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
        let first = execute_journaled_proof(&journal, &mut state, JournaledProofExecution { catalog: &catalog, name: "no-model", launcher: &launcher, receipts: &store, cancellation: &ProcessCancellation::default(), observed_at: "fixed".into() }).unwrap();
        assert_eq!(first.disposition, ProofDisposition::Passed); assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
        let second = execute_journaled_proof(&journal, &mut journal.rebuild().unwrap(), JournaledProofExecution { catalog: &catalog, name: "no-model", launcher: &Panic, receipts: &store, cancellation: &ProcessCancellation::default(), observed_at: "ignored".into() }).unwrap();
        assert_eq!(first, second);
    }

    fn recovery_content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    #[test]
    fn recovery_reconciles_foreign_bytes_and_certifies_strict_descent() {
        let policy = sim_roadmap_exec_core::RecoveryPolicy {
            max_refinement_rank: 4,
            ..Default::default()
        };
        for n in 0..512 {
            let ambiguous = ResumeDecision::Ambiguous {
                foreign_paths: vec![format!("foreign-{n}.rs")],
            };
            let effect = plan_refinement_after_reconciliation(
                &ambiguous,
                true,
                recovery_content((n % 250 + 1) as u8),
                3,
                2,
                &policy,
            );
            assert!(matches!(
                effect,
                RecoveryEffect::Stop(RecoveryStop::Ambiguous { .. })
            ));
            assert!(terminal_requests_no_effects(&effect));
        }
        let profile = recovery_content(7);
        assert_eq!(
            plan_refinement_after_reconciliation(
                &ResumeDecision::Committed,
                true,
                profile.clone(),
                4,
                3,
                &policy,
            ),
            RecoveryEffect::InvokeRefiner {
                derived_profile: profile
            }
        );
        for (fresh, parent, child) in [(false, 4, 3), (true, 4, 4), (true, 4, 5)] {
            assert!(terminal_requests_no_effects(
                &plan_refinement_after_reconciliation(
                    &ResumeDecision::Committed,
                    fresh,
                    recovery_content(8),
                    parent,
                    child,
                    &policy,
                )
            ));
        }
    }
