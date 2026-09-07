    use sim_kernel::{ContentId, Datum, Lib, Symbol};
    use sim_lib_journal::{
        Admission, JournalBackend, JournalError, JournalHead, JournalObject, Lease, MemoryBackend,
        StoredDatumRef, StoredState,
    };
    use sim_lib_roadmap_runner::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn proof_content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    fn local_request(verb: &str, observe: bool) -> LocalRoadmapRequest {
        LocalRoadmapRequest {
            verb: verb.into(),
            observe,
            disposable_checkout: None,
            local_authority_token: None,
            identity: LocalExecutionIdentity {
                execution: "local/specimen".into(),
                conduct: "sha256:conduct".into(),
                model_pick: "sha256:no-model".into(),
                proof_catalog: "sha256:proofs".into(),
                runner_generation: "sha256:generation".into(),
            },
        }
    }

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
    fn stale_writers_duplicates_budgets_and_identity_changes_fail_closed() {
        let log = ExecutionJournal::new(
            Arc::new(MemoryBackend::new()),
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
        fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
            self.inner.put_datum(object)
        }
        fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
            self.inner.get_datum(meaning)
        }
        fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
            self.inner.rebuild_datum_index()
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
        fn put_datum(&self, _: JournalObject) -> Result<StoredDatumRef, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
        fn get_datum(&self, _: &ContentId) -> Result<Datum, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
        fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
            Ok(Vec::new())
        }
    }
