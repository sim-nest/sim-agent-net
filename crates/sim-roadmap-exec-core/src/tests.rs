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
