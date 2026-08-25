#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use sim_kernel::{ContentId, Symbol};
    use sim_roadmap_core::{PhaseId, PromiseId};
    use sim_roadmap_exec_core::{
        reduce, AttemptId, ExecutionEvent, ExecutionFailure, ExecutionId, ExecutionPolicy,
        ExecutionPolicyId, FileImage, MutationId, MutationPlan, Observation, PhaseRunState,
        PromiseDischarge, Transition,
    };

    fn content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    #[test]
    fn forged_success_cannot_mint_a_receipt() {
        let execution = ExecutionId::new("focused").unwrap();
        let phase = PhaseId::new("phase").unwrap();
        let attempt = AttemptId::new("attempt").unwrap();
        let policy = ExecutionPolicy {
            id: ExecutionPolicyId::new("policy").unwrap(),
            source_deck: content(9),
            required_promises: vec![PromiseId::new("tests").unwrap()],
            required_proofs: vec![Symbol::new("tests")],
        };
        let plan = MutationPlan::new(
            MutationId::new("mutation").unwrap(),
            vec![FileImage {
                path: "src/lib.rs".into(),
                content: Some(content(1)),
            }],
            vec![FileImage {
                path: "src/lib.rs".into(),
                content: Some(content(2)),
            }],
        )
        .unwrap();
        let mut state = Transition {
            state: PhaseRunState::Reconciling,
            journal_head: content(3),
            ..Transition::default()
        };
        let succeed = |head| ExecutionEvent {
            execution: execution.clone(),
            phase: phase.clone(),
            attempt: attempt.clone(),
            observation: Observation {
                kind: Symbol::new("succeed"),
                journal_head: content(head),
                ..Observation::default()
            },
        };

        assert!(matches!(
            reduce(
                &policy,
                &plan,
                &execution,
                &phase,
                &attempt,
                &state,
                &succeed(4)
            ),
            Err(ExecutionFailure::SuccessInvariant(_))
        ));
        assert!(state.receipt.is_none());

        state.committed_postimages = plan.postimages.clone();
        state.current_source_deck = Some(policy.source_deck.clone());
        state.parent_acceptance_retained = true;
        state.discharges.push(PromiseDischarge {
            promise: policy.required_promises[0].clone(),
            status: Symbol::new("proven"),
            evidence: Some(content(5)),
        });
        let completed = reduce(
            &policy,
            &plan,
            &execution,
            &phase,
            &attempt,
            &state,
            &succeed(6),
        )
        .unwrap();
        assert_eq!(completed.state, PhaseRunState::Succeeded);
        assert!(completed.receipt.is_some());
    }
}
