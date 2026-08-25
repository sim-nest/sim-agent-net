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

#[cfg(test)]
mod proof_leaf_tests {
    use sim_lib_exec::*;
    use sim_lib_journal::MemoryBackend;
    use sim_lib_roadmap_runner::*;
    use sim_kernel::{ContentId, Symbol};
    use std::{collections::{BTreeMap, BTreeSet}, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};

    fn limits() -> SandboxLimits { SandboxLimits { cpu_seconds: 1, memory_bytes: 1_000_000,
        wall_time_ms: 50, process_count: 2, file_count: 2, file_bytes: 100, output_bytes: 100, stdin_bytes: 1 } }
    fn command(name: &str) -> CommandProof { CommandProof { name: name.into(), effect_id: format!("effect-{name}"),
        program: "tool:harmless".into(), argv: vec!["literal;not-shell".into()], working_directory: "/source".into(),
        environment: BTreeMap::new(), allowed_environment_keys: BTreeSet::new(), source_mount: "source:deck".into(),
        scratch_mount: Some("scratch:proof".into()), source_read_only: true, limits: limits(),
        expected: StructuredExpectation { stdout_sha256: "2689367b205c16ce32ed4200942b8b8b1e262dfc70d9bc9fbc77c49699a4f1df".into(), exit_code: 0 } } }
    fn report(request: &SandboxRequest, hits: Vec<String>) -> SandboxReport { SandboxReport {
        launcher: "fixture".into(), controls: request.policy.requirements().keys().map(|control| SandboxEvidence {
            control: *control, achieved: true, detail: "isolated".into() }).collect(), limit_hits: hits, cleanup: "sandbox:bounded/reaped".into() } }
    struct Launcher { calls: AtomicUsize, mode: &'static str }
    impl SandboxLauncher for Launcher {
        fn id(&self) -> &str { "fixture" }
        fn launch(&self, request: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.environment.iter().count(), 1);
            assert_eq!(request.policy.mounts()[0].access, MountAccess::ReadOnly);
            assert_eq!(request.policy.requirements()[&SandboxControl::Network], SandboxRequirement::Required);
            match self.mode {
                "ok" => SandboxAttempt::Completed(SandboxResult { stdout: b"ok".to_vec(), stderr: vec![], exit_code: 0, report: report(request, vec![]) }),
                "flood" => SandboxAttempt::Completed(SandboxResult { stdout: vec![b'x'; 100], stderr: vec![], exit_code: 0, report: report(request, vec!["output".into()]) }),
                "fork" => SandboxAttempt::Stopped(report(request, vec!["process-count".into()])),
                "timeout" => SandboxAttempt::Stopped(report(request, vec!["wall-time".into()])),
                "write" | "network" => SandboxAttempt::Refused(SandboxRefusal { launcher: "fixture".into(), reason: self.mode.into(), report: Some(report(request, vec![])) }),
                _ => SandboxAttempt::Completed(SandboxResult { stdout: b"malformed".to_vec(), stderr: vec![], exit_code: 0, report: report(request, vec![]) }),
            }
        }
    }
    struct Panic;
    impl SandboxLauncher for Panic { fn id(&self) -> &str { "panic" } fn launch(&self, _: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt { panic!("effect during replay") } }
    #[derive(Default)] struct Store(Mutex<BTreeMap<String, TypedProofReceipt>>);
    impl ProofReceiptStore for Store { fn inspect(&self, id: &str) -> Option<TypedProofReceipt> { self.0.lock().unwrap().get(id).cloned() }
        fn record(&self, id: &str, receipt: &TypedProofReceipt) { self.0.lock().unwrap().insert(id.into(), receipt.clone()); } }

    #[test]
    fn hostile_catalog_and_sandbox_are_fail_closed() {
        let mut invalid = command("shell"); invalid.program = "sh -c env".into();
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("ambient"); invalid.environment.insert("HOME".into(), "/host".into());
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("secret"); invalid.environment.insert("API_TOKEN".into(), "steal".into()); invalid.allowed_environment_keys.insert("API_TOKEN".into());
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("absolute"); invalid.source_mount = "/host/source".into();
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("writeable"); invalid.source_read_only = false;
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        for mode in ["flood", "fork", "timeout", "malformed", "write", "network"] {
            let catalog = ProofCatalog::new([ProofLeaf::Command(command(mode))]).unwrap();
            let receipt = execute_proof_leaf(&catalog, mode, &Launcher { calls: AtomicUsize::new(0), mode }, &ProcessCancellation::default(), "fixed").unwrap();
            assert_ne!(receipt.disposition, ProofDisposition::Passed, "{mode}");
        }
    }

    #[test]
    fn pure_deck_and_artifact_proofs_never_launch() {
        let catalog = ProofCatalog::new([ProofLeaf::ArtifactEquality { name: "artifact".into(), left: b"x".to_vec(), right: b"x".to_vec() },
            ProofLeaf::SourceDeckPredicate { name: "deck".into(), actual_deck: "deck:1".into(), expected_deck: "deck:1".into(),
                required_claims: BTreeSet::from(["claim".into()]), present_claims: BTreeSet::from(["claim".into()]) }]).unwrap();
        for name in ["artifact", "deck"] { assert_eq!(execute_proof_leaf(&catalog, name, &Panic, &ProcessCancellation::default(), "fixed").unwrap().disposition, ProofDisposition::Passed); }
    }

    #[test]
    fn recorded_no_model_command_replays_without_launch() {
        let catalog = ProofCatalog::new([ProofLeaf::Command(command("no-model"))]).unwrap();
        let journal = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "proof", Limits::default());
        let pins = ExecutionPins { conduct: "conduct".into(), policy: "policy".into(), source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [1; 32]), model_pick: "none".into(), runner_generation: "runner".into() };
        let mut state = journal.open(pins, None).unwrap(); let store = Store::default();
        let launcher = Launcher { calls: AtomicUsize::new(0), mode: "ok" };
        let first = execute_journaled_proof(&journal, &mut state, &catalog, "no-model", &launcher, &store, &ProcessCancellation::default(), "fixed").unwrap();
        assert_eq!(first.disposition, ProofDisposition::Passed); assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
        let second = execute_journaled_proof(&journal, &mut journal.rebuild().unwrap(), &catalog, "no-model", &Panic, &store, &ProcessCancellation::default(), "ignored").unwrap();
        assert_eq!(first, second);
    }
}
