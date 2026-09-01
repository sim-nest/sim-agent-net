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
