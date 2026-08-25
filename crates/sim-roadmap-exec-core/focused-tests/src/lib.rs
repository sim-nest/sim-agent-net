#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use sim_kernel::{ContentId, Symbol};
    use sim_roadmap_core::{PhaseId, PromiseId};
    use sim_roadmap_exec_core::{
        admit_model_fallback, admit_retry, reduce, AttemptId, ClassifiedFailure, ExecutionEvent,
        ExecutionFailure, ExecutionId, ExecutionPolicy, ExecutionPolicyId, FailureClass, FileImage,
        ModelPickRecord, MutationId, MutationPlan, Observation, PhaseRunState, PromiseDischarge,
        RecoveryPolicy, RetryContext, RetryDecision, RetryRule, StopReason, Transition,
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

    #[test]
    fn randomized_retries_are_finite_and_model_fallback_is_pinned() {
        let mut seed = 0x5eed_u64;
        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let bound = (seed % 8) as u32;
            let mut policy = RecoveryPolicy {
                max_child_attempts: 1,
                ..Default::default()
            };
            policy.retry.insert(
                FailureClass::InfrastructureTransient,
                RetryRule {
                    max_attempts: bound,
                    backoff_millis: vec![],
                },
            );
            let failure = ClassifiedFailure {
                class: FailureClass::InfrastructureTransient,
                evidence: vec![content(1)],
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
                        identities_before: vec![content(2)],
                        identities_now: vec![content(2)],
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
            let pick = ModelPickRecord {
                record_id: content(3),
                primary: content(4),
                compatible_fallbacks: vec![content(5)],
            };
            assert!(admit_model_fallback(
                &policy,
                &pick,
                &content(4),
                &content(5),
                &AttemptId::new("parent").unwrap(),
                AttemptId::new("child").unwrap(),
                0,
                vec![content(6)],
            ).is_some());
            assert!(admit_model_fallback(
                &policy,
                &pick,
                &content(4),
                &content(9),
                &AttemptId::new("parent").unwrap(),
                AttemptId::new("foreign").unwrap(),
                0,
                vec![],
            ).is_none());
        }
    }
}
