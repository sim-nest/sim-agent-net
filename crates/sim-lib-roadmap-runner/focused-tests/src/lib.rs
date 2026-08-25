#[cfg(test)]
mod tests {
    use sim_kernel::{ContentId, Symbol};
    use sim_lib_journal::{
        Admission, JournalBackend, JournalError, JournalHead, Lease, MemoryBackend, StoredState,
    };
    use sim_lib_roadmap_runner::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn pins(n: u8) -> ExecutionPins {
        ExecutionPins {
            conduct: format!("conduct-{n}"),
            policy: format!("policy-{n}"),
            source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [n; 32]),
            model_pick: format!("model-{n}"),
            runner_generation: format!("runner-{n}"),
        }
    }

    #[test]
    fn receipt_only_replay_is_exact_and_performs_no_effects() {
        let log = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "exec", Limits::default());
        let mut state = log.open(pins(1), None).unwrap();
        let packet = log
            .prepare_object(
                ObjectKind::Packet,
                b"bounded packet".to_vec(),
                "packet summary",
            )
            .unwrap();
        for (record, objects) in [
            (
                ExecutionRecord::StateTransition {
                    from: "planned".into(),
                    to: "running".into(),
                },
                vec![],
            ),
            (
                ExecutionRecord::EffectRequested {
                    effect_id: "effect-1".into(),
                    kind: "process".into(),
                    input: Some(packet.reference.clone()),
                },
                vec![packet],
            ),
        ] {
            state.head = log.append(Some(&state.head), record, objects).unwrap();
        }
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
        assert_eq!(log.rebuild().unwrap(), log.rebuild().unwrap());
        assert_eq!(log.rebuild().unwrap().records.len(), 11);
    }

    #[test]
    fn stale_writers_duplicate_receipts_secrets_limits_and_pin_drift_fail_closed() {
        let log = ExecutionJournal::new(
            Arc::new(MemoryBackend::new()),
            "exec",
            Limits {
                max_object_bytes: 64,
                ..Limits::default()
            },
        );
        let opened = log.open(pins(1), None).unwrap();
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
            log.prepare_object(ObjectKind::FileBytes, vec![0; 65], "safe"),
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

    struct InspectBackend {
        inner: MemoryBackend,
        crash: AtomicBool,
    }
    impl Default for InspectBackend {
        fn default() -> Self {
            Self {
                inner: MemoryBackend::new(),
                crash: AtomicBool::new(false),
            }
        }
    }
    impl JournalBackend for InspectBackend {
        fn acquire_lease(&self) -> Result<Lease, JournalError> {
            self.inner.acquire_lease()
        }
        fn read_state(&self) -> Result<StoredState, JournalError> {
            self.inner.read_state()
        }
        fn admit(&self, a: Admission) -> Result<JournalHead, JournalError> {
            if self.crash.swap(false, Ordering::SeqCst) {
                Err(JournalError::InjectedCrash("adapter-admission"))
            } else {
                self.inner.admit(a)
            }
        }
    }
    struct SnapshotBackend(StoredState);
    impl JournalBackend for SnapshotBackend {
        fn acquire_lease(&self) -> Result<Lease, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
        fn read_state(&self) -> Result<StoredState, JournalError> {
            Ok(self.0.clone())
        }
        fn admit(&self, _: Admission) -> Result<JournalHead, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
    }

    #[test]
    fn crash_missing_object_truncated_reordered_and_cross_run_history_report_last_verified_head() {
        let backend = Arc::new(InspectBackend::default());
        let log = ExecutionJournal::new(backend.clone(), "exec", Limits::default());
        let opened = log.open(pins(1), None).unwrap();
        let object = log
            .prepare_object(ObjectKind::FileBytes, b"bytes".to_vec(), "file")
            .unwrap();
        backend.crash.store(true, Ordering::SeqCst);
        assert!(matches!(
            log.append(
                Some(&opened.head),
                ExecutionRecord::EffectRequested {
                    effect_id: "write".into(),
                    kind: "file".into(),
                    input: Some(object.reference.clone())
                },
                vec![object]
            ),
            Err(ExecutionJournalError::Journal(JournalError::InjectedCrash(
                _
            )))
        ));
        assert_eq!(log.rebuild().unwrap().head, opened.head);
        let head = log
            .append(
                Some(&opened.head),
                ExecutionRecord::Ambiguity {
                    reason: "recover".into(),
                },
                vec![],
            )
            .unwrap();
        let state = backend.read_state().unwrap();
        let mut missing = state.clone();
        missing.objects.clear();
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(missing)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert!(report.last_verified_head.is_none());
        let mut truncated = state.clone();
        truncated.entries.remove(&head.sequence);
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(truncated)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert_eq!(report.last_verified_head, Some(opened.head.clone()));
        let mut reordered = state.clone();
        reordered.entries.get_mut(&1).unwrap().sequence = 2;
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(reordered)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert_eq!(report.last_verified_head, Some(opened.head));
        assert!(matches!(
            ExecutionJournal::new(
                Arc::new(SnapshotBackend(state)),
                "foreign-exec",
                Limits::default()
            )
            .rebuild_report()
            .unwrap_err()
            .error,
            ExecutionJournalError::ExecutionIdentity
        ));
    }
}
