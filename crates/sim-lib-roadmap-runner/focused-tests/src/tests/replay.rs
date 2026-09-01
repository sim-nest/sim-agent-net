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
