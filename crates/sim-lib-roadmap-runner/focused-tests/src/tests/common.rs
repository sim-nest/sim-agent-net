    use sim_kernel::{ContentId, Lib, Symbol};
    use sim_lib_journal::{
        Admission, JournalBackend, JournalError, JournalHead, Lease, MemoryBackend, StoredState,
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
