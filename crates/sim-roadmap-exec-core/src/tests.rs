use sim_citizen::{CitizenRegistry, run_registry_conformance_expecting};
use sim_kernel::{ContentId, Datum, Expr, Symbol, testing::bare_cx as cx};
use sim_roadmap_core::{PhaseId, PromiseId};

use crate::*;

fn cid(n: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32])
}
fn fixture() -> (
    ExecutionPolicy,
    MutationPlan,
    ExecutionId,
    PhaseId,
    AttemptId,
    Transition,
) {
    let policy = ExecutionPolicy {
        id: ExecutionPolicyId::new("policy").unwrap(),
        source_deck: cid(9),
        required_promises: vec![PromiseId::new("api").unwrap()],
        required_proofs: vec![Symbol::new("tests")],
    };
    let plan = MutationPlan::new(
        MutationId::new("mutation").unwrap(),
        vec![FileImage {
            path: "src/lib.rs".into(),
            content: Some(cid(1)),
        }],
        vec![FileImage {
            path: "src/lib.rs".into(),
            content: Some(cid(2)),
        }],
    )
    .unwrap();
    let execution = ExecutionId::new("execution").unwrap();
    let phase = PhaseId::new("phase").unwrap();
    let attempt = AttemptId::new("attempt").unwrap();
    let transition = Transition {
        journal_head: cid(0),
        ..Transition::default()
    };
    (policy, plan, execution, phase, attempt, transition)
}
fn event(
    execution: &ExecutionId,
    phase: &PhaseId,
    attempt: &AttemptId,
    head: u8,
    kind: &str,
) -> ExecutionEvent {
    ExecutionEvent {
        execution: execution.clone(),
        phase: phase.clone(),
        attempt: attempt.clone(),
        observation: Observation {
            kind: Symbol::new(kind),
            journal_head: cid(head),
            ..Observation::default()
        },
    }
}

#[test]
fn citizens_supply_card_shape_codec_and_read_construct() {
    let mut registry = CitizenRegistry::new();
    register_citizens(&mut registry).unwrap();
    run_registry_conformance_expecting(&mut cx(), &registry, &["roadmap-exec/Value"]).unwrap();
    assert!(ExecutionId::new("x".repeat(10_000)).is_err());
}

#[test]
fn transition_matrix_covers_every_state_and_observation_category() {
    let (policy, plan, x, p, a, s) = fixture();
    let cases = [
        (PhaseRunState::Planned, "start", true),
        (PhaseRunState::Planned, "cancel", true),
        (PhaseRunState::Planned, "succeed", false),
        (PhaseRunState::Running, "image-observed", false),
        (PhaseRunState::Running, "mutation-committed", true),
        (PhaseRunState::Running, "fail", true),
        (PhaseRunState::Running, "cancel", true),
        (PhaseRunState::Running, "start", false),
        (PhaseRunState::Reconciling, "promise-discharged", false),
        (PhaseRunState::Reconciling, "proof-unresolved", false),
        (PhaseRunState::Reconciling, "source-deck-current", true),
        (PhaseRunState::Reconciling, "parent-accepted", true),
        (PhaseRunState::Reconciling, "succeed", false),
        (PhaseRunState::Reconciling, "fail", true),
        (PhaseRunState::Reconciling, "cancel", true),
        (PhaseRunState::Succeeded, "start", false),
        (PhaseRunState::Failed, "start", false),
        (PhaseRunState::Cancelled, "start", false),
    ];
    for (i, (state, kind, accepted)) in cases.into_iter().enumerate() {
        let mut before = s.clone();
        before.state = state;
        let result = reduce(
            &policy,
            &plan,
            &x,
            &p,
            &a,
            &before,
            &event(&x, &p, &a, (i + 1) as u8, kind),
        );
        assert_eq!(result.is_ok(), accepted, "{state:?} + {kind}");
    }
}

#[test]
fn replay_is_deterministic_and_effect_requests_are_data() {
    let (policy, plan, x, p, a, s) = fixture();
    let events = [event(&x, &p, &a, 1, "start")];
    let left = replay(&policy, &plan, &x, &p, &a, &s, &events).unwrap();
    let right = replay(&policy, &plan, &x, &p, &a, &s, &events).unwrap();
    assert_eq!(left, right);
    assert_eq!(
        left.requested_effects[0].kind,
        Symbol::new("apply-mutation")
    );
}

#[test]
fn every_correlation_axis_rejects_forgery() {
    let (policy, plan, x, p, a, s) = fixture();
    let good = event(&x, &p, &a, 1, "start");
    let mut forged = good.clone();
    forged.execution = ExecutionId::new("other").unwrap();
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongExecution)
    );
    let mut forged = good.clone();
    forged.phase = PhaseId::new("other").unwrap();
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongPhase)
    );
    let mut forged = good.clone();
    forged.attempt = AttemptId::new("other").unwrap();
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongAttempt)
    );
    let mut forged = good.clone();
    forged.observation.journal_head = s.journal_head.clone();
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongJournalHead)
    );
    let mut forged = good.clone();
    forged.observation.mutation = Some(MutationId::new("other").unwrap());
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongMutation)
    );
    let mut forged = good;
    forged.observation.proof_cursor = Some(ProofCursor {
        sequence: 1,
        journal_head: cid(7),
        proof: Symbol::new("tests"),
    });
    assert_eq!(
        reduce(&policy, &plan, &x, &p, &a, &s, &forged),
        Err(ExecutionFailure::WrongProofCursor)
    );
}

#[test]
fn forged_success_cannot_mint_receipt() {
    let (policy, plan, x, p, a, mut s) = fixture();
    s.state = PhaseRunState::Reconciling;
    let result = reduce(
        &policy,
        &plan,
        &x,
        &p,
        &a,
        &s,
        &event(&x, &p, &a, 1, "succeed"),
    );
    assert!(matches!(result, Err(ExecutionFailure::SuccessInvariant(_))));
    assert!(s.receipt.is_none());
    s.committed_postimages = plan.postimages.clone();
    s.current_source_deck = Some(policy.source_deck.clone());
    s.parent_acceptance_retained = true;
    s.discharges = vec![PromiseDischarge {
        promise: policy.required_promises[0].clone(),
        status: Symbol::new("proven"),
        evidence: Some(cid(8)),
    }];
    let done = reduce(
        &policy,
        &plan,
        &x,
        &p,
        &a,
        &s,
        &event(&x, &p, &a, 2, "succeed"),
    )
    .unwrap();
    assert_eq!(done.state, PhaseRunState::Succeeded);
    assert!(done.receipt.is_some());
}

#[test]
fn sorted_paths_classification_and_identity_are_stable_over_many_inputs() {
    for n in 1..64u8 {
        let a = FileImage {
            path: format!("src/{n}.rs"),
            content: Some(cid(n)),
        };
        let b = FileImage {
            path: format!("src/{}.rs", n + 1),
            content: Some(cid(n + 1)),
        };
        let p = MutationPlan::new(
            MutationId::new(format!("m{n}")).unwrap(),
            vec![b.clone(), a.clone()],
            vec![a.clone()],
        )
        .unwrap();
        assert!(p.preimages[0].path < p.preimages[1].path);
        assert_eq!(p.classify(&a), ImageClass::PreAndPost);
        assert_eq!(p.classify(&b), ImageClass::Preimage);
        assert_eq!(
            p.classify(&FileImage {
                path: "foreign".into(),
                content: None
            }),
            ImageClass::Foreign
        );
        let d = Datum::List(vec![
            Datum::String(p.id.to_string()),
            Datum::String(p.preimages[0].path.clone()),
        ]);
        assert_eq!(d.content_id().unwrap(), d.clone().content_id().unwrap());
    }
    assert_eq!(
        MutationPlan::new(
            MutationId::new("dup").unwrap(),
            vec![
                FileImage {
                    path: "a".into(),
                    content: None
                },
                FileImage {
                    path: "a".into(),
                    content: Some(cid(1))
                }
            ],
            vec![]
        ),
        Err(ExecutionFailure::DuplicatePath)
    );
}

#[test]
fn public_api_contains_no_effect_trait_or_adapter_handle() {
    let source =
        include_str!("lib.rs").to_owned() + include_str!("model.rs") + include_str!("reduce.rs");
    assert!(!source.contains("trait Effect"));
    for forbidden in ["std::fs", "std::process", "tokio::", "reqwest::", "git2::"] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn recovery_classes_have_exact_owner_and_unknown_stops() {
    let cases = [
        (
            FailureClass::DeterministicInput,
            FailureOwner::InputAuthor,
            false,
        ),
        (FailureClass::Conduct, FailureOwner::Conduct, true),
        (FailureClass::Mutation, FailureOwner::MutationAdapter, false),
        (FailureClass::Proof, FailureOwner::ProofSystem, true),
        (
            FailureClass::InfrastructureTransient,
            FailureOwner::Infrastructure,
            true,
        ),
        (
            FailureClass::InfrastructurePermanent,
            FailureOwner::Infrastructure,
            false,
        ),
        (FailureClass::Budget, FailureOwner::BudgetAuthority, false),
        (
            FailureClass::Authority,
            FailureOwner::AuthorityHolder,
            false,
        ),
        (FailureClass::Ambiguity, FailureOwner::Unknown, false),
    ];
    for (class, owner, retry_safe) in cases {
        assert_eq!(class.owner(), owner);
        assert_eq!(class.intrinsically_retry_safe(), retry_safe);
    }
    assert_eq!(
        ClassifiedFailure::unknown(vec![]).class,
        FailureClass::Ambiguity
    );
}

#[test]
fn recovery_retry_is_named_bounded_identity_stable_and_receipted() {
    let mut policy = RecoveryPolicy::default();
    policy.retry.insert(
        FailureClass::InfrastructureTransient,
        RetryRule {
            max_attempts: 2,
            backoff_millis: vec![5, 13],
        },
    );
    let failure = ClassifiedFailure {
        class: FailureClass::InfrastructureTransient,
        evidence: vec![cid(1)],
    };
    for used in 0..=2 {
        let decision = admit_retry(
            &policy,
            &failure,
            &RetryContext {
                attempt: AttemptId::new("parent").unwrap(),
                attempts_used: used,
                unresolved_effect: false,
                identities_before: vec![cid(2)],
                identities_now: vec![cid(2)],
            },
        );
        if used < 2 {
            let RetryDecision::Retry(receipt) = decision else {
                panic!("retry expected")
            };
            assert_eq!(receipt.next_attempt_number, used + 1);
            assert_eq!(receipt.remaining_attempts, 1 - used);
            assert_eq!(receipt.unchanged_identities, vec![cid(2)]);
        } else {
            assert_eq!(decision, RetryDecision::Stop(StopReason::AttemptsExhausted));
        }
    }
}

#[test]
fn recovery_fallback_is_pinned_child_and_preserves_failed_evidence() {
    let policy = RecoveryPolicy {
        max_child_attempts: 1,
        ..RecoveryPolicy::default()
    };
    let pick = ModelPickRecord {
        record_id: cid(1),
        primary: cid(2),
        compatible_fallbacks: vec![cid(3)],
    };
    let child = admit_model_fallback(
        &policy,
        &pick,
        &cid(2),
        &cid(3),
        &AttemptId::new("parent").unwrap(),
        AttemptId::new("child").unwrap(),
        0,
        vec![cid(4)],
    )
    .unwrap();
    assert_eq!(child.pick_record, cid(1));
    assert_eq!(child.failed_evidence_retained, vec![cid(4)]);
    assert!(
        admit_model_fallback(
            &policy,
            &pick,
            &cid(2),
            &cid(9),
            &child.parent,
            AttemptId::new("foreign").unwrap(),
            0,
            vec![]
        )
        .is_none()
    );
}

#[test]
fn randomized_recovery_sequences_converge_within_declared_bound() {
    let mut seed = 0x5eed_u64;
    for _case in 0..2_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let bound = (seed % 8) as u32;
        let mut policy = RecoveryPolicy::default();
        policy.retry.insert(
            FailureClass::InfrastructureTransient,
            RetryRule {
                max_attempts: bound,
                backoff_millis: vec![],
            },
        );
        let failure = ClassifiedFailure {
            class: FailureClass::InfrastructureTransient,
            evidence: vec![],
        };
        let mut used = 0;
        loop {
            match admit_retry(
                &policy,
                &failure,
                &RetryContext {
                    attempt: AttemptId::new(format!("attempt-{used}")).unwrap(),
                    attempts_used: used,
                    unresolved_effect: false,
                    identities_before: vec![cid(1)],
                    identities_now: vec![cid(1)],
                },
            ) {
                RetryDecision::Retry(receipt) => {
                    assert_eq!(receipt.next_attempt_number, used + 1);
                    assert!(receipt.next_attempt_number <= bound);
                    used += 1;
                }
                RetryDecision::Stop(reason) => {
                    assert_eq!(reason, StopReason::AttemptsExhausted);
                    assert_eq!(used, bound);
                    break;
                }
            }
        }
    }
}

#[test]
fn recovery_escalation_is_bounded_and_redacted() {
    let card = EscalationCard {
        verified_state: vec!["<packet raw>".into(), "clean".into()],
        safe_paths: vec!["secret token".into()],
        evidence_ids: vec![cid(1)],
        missing_authority_or_decision: "human choice".into(),
        permitted_next_actions: (0..32).map(|n| format!("action-{n}")).collect(),
    };
    let rendered = card.render_redacted();
    assert!(!rendered.contains("raw"));
    assert!(!rendered.contains("token"));
    assert_eq!(
        rendered.matches("permitted:").count(),
        EscalationCard::MAX_ROWS
    );
}
