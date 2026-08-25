// conformance: durable roadmap runner journal and replay laws.

use std::sync::Arc;

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::{JournalError, MemoryBackend};

use crate::*;

fn pins(n: u8) -> ExecutionPins {
    ExecutionPins {
        conduct: format!("conduct-{n}"),
        policy: format!("policy-{n}"),
        source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [n; 32]),
        model_pick: format!("model-{n}"),
        runner_generation: format!("runner-{n}"),
    }
}

mod bounded_service {
    use std::sync::{Arc, Mutex};

    use sim_kernel::{ContentId, Symbol};
    use sim_lib_journal::MemoryBackend;
    use sim_roadmap_core::PhaseId;
    use sim_roadmap_exec_core::{
        AttemptId, ExecutionEvent, ExecutionId, ExecutionPolicy, ExecutionPolicyId, MutationId,
        MutationPlan, Observation, PhaseRunState,
    };

    use crate::*;

    fn cid(n: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32])
    }
    fn ceiling(amount: u64) -> EffectiveCeiling {
        EffectiveCeiling::intersect([
            OwnedLimit {
                owner: Symbol::new("caller"),
                unit: Symbol::new("model-tokens"),
                amount,
            },
            OwnedLimit {
                owner: Symbol::new("roadmap"),
                unit: Symbol::new("model-tokens"),
                amount: 50,
            },
            OwnedLimit {
                owner: Symbol::new("sandbox"),
                unit: Symbol::new("processes"),
                amount: 1,
            },
        ])
    }
    fn request(grant: u8, budget: u64) -> OpenRequest {
        let policy = ExecutionPolicy {
            id: ExecutionPolicyId::new("policy").unwrap(),
            source_deck: cid(2),
            required_promises: vec![],
            required_proofs: vec![],
        };
        OpenRequest {
            authority: AuthorityGrant {
                identity: ExecutionIdentity {
                    execution: ExecutionId::new("execution").unwrap(),
                    policy: policy.id.clone(),
                    roadmap: cid(1),
                    source_deck: policy.source_deck.clone(),
                    conduct: cid(3),
                    model: cid(4),
                    launcher: cid(5),
                    runner: cid(6),
                },
                ceiling: ceiling(budget),
                grant: cid(grant),
            },
            phase: PhaseId::new("leaf").unwrap(),
            attempt: AttemptId::new("attempt").unwrap(),
            policy,
            mutation: MutationPlan::new(MutationId::new("mutation").unwrap(), vec![], vec![])
                .unwrap(),
        }
    }
    struct Ready(PhaseId);
    impl ReadinessPort for Ready {
        fn admitted_leaf(&self, _: &ContentId) -> Result<Option<PhaseId>, ServiceError> {
            Ok(Some(self.0.clone()))
        }
    }
    #[derive(Default)]
    struct NeverCancel;
    impl CancellationPort for NeverCancel {
        fn cancellation_requested(&self, _: &ExecutionId) -> bool {
            false
        }
    }
    struct FakeEffects(Mutex<Vec<ExecutionEvent>>);
    impl EffectPort for FakeEffects {
        fn invoke(
            &self,
            identity: &ExecutionIdentity,
            phase: &PhaseId,
            _: &sim_roadmap_exec_core::Transition,
        ) -> Result<ExecutionEvent, ServiceError> {
            let event = ExecutionEvent {
                execution: identity.execution.clone(),
                phase: phase.clone(),
                attempt: AttemptId::new("attempt").unwrap(),
                observation: Observation {
                    kind: Symbol::new("start"),
                    journal_head: cid(20),
                    ..Default::default()
                },
            };
            self.0.lock().unwrap().push(event.clone());
            Ok(event)
        }
        fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    #[test]
    fn one_advance_journals_intent_then_receipt_and_replay_is_byte_equivalent() {
        let request = request(10, 100);
        let mut service = RoadmapRunnerService::open(
            Arc::new(MemoryBackend::new()),
            Ready(request.phase.clone()),
            FakeEffects(Mutex::new(vec![])),
            NeverCancel,
            request,
            Limits::default(),
        )
        .unwrap();
        let inspected = service.advance_one_effect().unwrap();
        assert_eq!(inspected.transition.state, PhaseRunState::Running);
        assert!(matches!(
            inspected.journal.records[1],
            ExecutionRecord::EffectRequested { .. }
        ));
        assert!(matches!(
            inspected.journal.records[2],
            ExecutionRecord::EffectReceipt { .. }
        ));
        assert_eq!(service.replay().unwrap(), inspected.transition);
    }

    #[test]
    fn pins_and_budget_provenance_fail_closed_before_an_effect() {
        let request = request(10, 100);
        assert_eq!(request.authority.ceiling.limits[0].sources.len(), 2);
        let mut service = RoadmapRunnerService::open(
            Arc::new(MemoryBackend::new()),
            Ready(request.phase.clone()),
            FakeEffects(Mutex::new(vec![])),
            NeverCancel,
            request.clone(),
            Limits::default(),
        )
        .unwrap();
        let mut replaced = request(11, 40).authority;
        replaced.identity.model = cid(99);
        assert!(matches!(
            service.resume(replaced),
            Err(ServiceError::IdentityDrift)
        ));
        let widened = request(11, 200).authority;
        assert!(matches!(
            service.resume(widened),
            Err(ServiceError::BudgetWidening)
        ));
        assert_eq!(service.inspect().unwrap().journal.records.len(), 1);
    }
}

#[test]
fn complete_record_family_replays_exactly_without_effects() {
    let backend = Arc::new(MemoryBackend::new());
    let log = ExecutionJournal::new(backend, "exec", Limits::default());
    let mut state = log.open(pins(1), None).unwrap();
    let packet = log
        .prepare_object(
            ObjectKind::Packet,
            b"bounded packet".to_vec(),
            "packet summary",
        )
        .unwrap();
    state.head = log
        .append(
            Some(&state.head),
            ExecutionRecord::StateTransition {
                from: "planned".into(),
                to: "running".into(),
            },
            vec![],
        )
        .unwrap();
    state.head = log
        .append(
            Some(&state.head),
            ExecutionRecord::EffectRequested {
                effect_id: "effect-1".into(),
                kind: "process".into(),
                input: Some(packet.reference.clone()),
            },
            vec![packet],
        )
        .unwrap();
    let output = log
        .prepare_object(ObjectKind::ProcessOutput, b"ok".to_vec(), "exit zero")
        .unwrap();
    state.head = log
        .append(
            Some(&state.head),
            ExecutionRecord::EffectReceipt {
                effect_id: "effect-1".into(),
                outcome: "ok".into(),
                output: Some(output.reference.clone()),
            },
            vec![output],
        )
        .unwrap();
    for record in [
        ExecutionRecord::MutationFence {
            mutation_id: "m1".into(),
            expected: "preimage".into(),
        },
        ExecutionRecord::ProofResult {
            proof: "tests".into(),
            passed: true,
            evidence: None,
        },
        ExecutionRecord::Discharge {
            obligation: "tests".into(),
        },
        ExecutionRecord::Ambiguity {
            reason: "none".into(),
        },
        ExecutionRecord::StateTransition {
            from: "running".into(),
            to: "reconciling".into(),
        },
        ExecutionRecord::StateTransition {
            from: "reconciling".into(),
            to: "succeeded".into(),
        },
        ExecutionRecord::TerminalReceipt {
            outcome: "succeeded".into(),
        },
    ] {
        state.head = log.append(Some(&state.head), record, vec![]).unwrap();
    }
    let left = log.rebuild().unwrap();
    let right = log.rebuild().unwrap();
    assert_eq!(left, right);
    assert_eq!(left.records.len(), 11);
}

#[test]
fn fences_duplicates_redaction_budgets_and_identity_changes_fail_closed() {
    let backend = Arc::new(MemoryBackend::new());
    let log = ExecutionJournal::new(
        backend,
        "exec",
        Limits {
            max_object_bytes: 8,
            ..Limits::default()
        },
    );
    let opened = log.open(pins(1), None).unwrap();
    assert!(matches!(
        log.prepare_object(ObjectKind::Packet, b"secret=oops".to_vec(), "packet"),
        Err(ExecutionJournalError::Budget("object"))
    ));
    assert!(matches!(
        log.prepare_object(ObjectKind::Packet, b"password=x".to_vec(), "packet"),
        Err(ExecutionJournalError::Budget("object"))
    ));
    let head = log
        .append(
            Some(&opened.head),
            ExecutionRecord::EffectRequested {
                effect_id: "x".into(),
                kind: "write".into(),
                input: None,
            },
            vec![],
        )
        .unwrap();
    assert!(matches!(
        log.append(
            Some(&opened.head),
            ExecutionRecord::Ambiguity {
                reason: "stale".into()
            },
            vec![]
        ),
        Err(ExecutionJournalError::Journal(
            JournalError::WrongHead | JournalError::ConflictingDelivery
        ))
    ));
    let receipt = ExecutionRecord::EffectReceipt {
        effect_id: "x".into(),
        outcome: "ok".into(),
        output: None,
    };
    let head = log.append(Some(&head), receipt.clone(), vec![]).unwrap();
    assert!(matches!(
        log.append(Some(&head), receipt, vec![]),
        Err(ExecutionJournalError::Illegal { .. })
    ));
    assert!(matches!(
        log.open(pins(2), Some(&head)),
        Err(ExecutionJournalError::ChildRequired { .. })
    ));
}

#[test]
fn secret_shaped_environment_and_packet_data_never_reach_objects() {
    let log = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "exec", Limits::default());
    for bytes in [
        b"API_KEY=abc".as_slice(),
        b"Authorization: Bearer abc".as_slice(),
        b"-----PRIVATE KEY-----".as_slice(),
    ] {
        assert!(matches!(
            log.prepare_object(ObjectKind::Packet, bytes.to_vec(), "safe"),
            Err(ExecutionJournalError::Secret)
        ));
    }
    assert!(matches!(
        log.prepare_object(ObjectKind::FileBytes, b"safe".to_vec(), "password=hunter2"),
        Err(ExecutionJournalError::Secret)
    ));
}

fn recovery_cid(n: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32])
}

#[test]
fn recovery_reconciles_before_every_effect_and_ambiguous_is_terminal() {
    let policy = sim_roadmap_exec_core::RecoveryPolicy::default();
    let failure = sim_roadmap_exec_core::ClassifiedFailure::unknown(vec![]);
    let context = sim_roadmap_exec_core::RetryContext {
        attempt: sim_roadmap_exec_core::AttemptId::new("a").unwrap(),
        attempts_used: 0,
        unresolved_effect: false,
        identities_before: vec![],
        identities_now: vec![],
    };
    let ambiguous = ResumeDecision::Ambiguous {
        foreign_paths: vec!["foreign.rs".into()],
    };
    let retry = plan_retry_after_reconciliation(&ambiguous, &policy, &failure, &context);
    assert!(matches!(
        retry,
        RecoveryEffect::Stop(RecoveryStop::Ambiguous { .. })
    ));
    assert!(terminal_requests_no_effects(&retry));
    let refine =
        plan_refinement_after_reconciliation(&ambiguous, true, recovery_cid(8), 3, 2, &policy);
    assert!(terminal_requests_no_effects(&refine));
}

#[test]
fn needs_refinement_requires_fresh_strict_bounded_descent() {
    let policy = sim_roadmap_exec_core::RecoveryPolicy {
        max_refinement_rank: 4,
        ..Default::default()
    };
    let profile = recovery_cid(7);
    assert_eq!(
        plan_refinement_after_reconciliation(
            &ResumeDecision::Committed,
            true,
            profile.clone(),
            4,
            3,
            &policy,
        ),
        RecoveryEffect::InvokeRefiner {
            derived_profile: profile
        }
    );
    for (fresh, parent, child) in [(false, 4, 3), (true, 4, 4), (true, 4, 5)] {
        let effect = plan_refinement_after_reconciliation(
            &ResumeDecision::Committed,
            fresh,
            recovery_cid(7),
            parent,
            child,
            &policy,
        );
        assert!(terminal_requests_no_effects(&effect));
    }
}

#[test]
fn foreign_bytes_never_request_mutation_in_randomized_sequences() {
    for n in 0..512 {
        let decision = ResumeDecision::Ambiguous {
            foreign_paths: vec![format!("foreign-{n}.rs")],
        };
        let effect = plan_refinement_after_reconciliation(
            &decision,
            true,
            recovery_cid((n % 250 + 1) as u8),
            2,
            1,
            &sim_roadmap_exec_core::RecoveryPolicy {
                max_refinement_rank: 1,
                ..Default::default()
            },
        );
        assert!(matches!(
            effect,
            RecoveryEffect::Stop(RecoveryStop::Ambiguous { .. })
        ));
        assert!(terminal_requests_no_effects(&effect));
    }
}
