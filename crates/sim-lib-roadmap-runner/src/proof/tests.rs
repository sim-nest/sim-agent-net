use super::*;
use crate::{ExecutionPins, Limits};
use sim_kernel::{ContentId, Symbol};
use sim_lib_exec::{SandboxEvidence, SandboxReport, SandboxResult};
use sim_lib_journal::MemoryBackend;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn limits() -> SandboxLimits {
    SandboxLimits {
        cpu_seconds: 1,
        memory_bytes: 32 * 1024 * 1024,
        wall_time_ms: 100,
        process_count: 2,
        file_count: 2,
        file_bytes: 1024,
        output_bytes: 1024,
        stdin_bytes: 1,
    }
}
fn command(name: &str, output: &[u8]) -> CommandProof {
    CommandProof {
        name: name.into(),
        effect_id: format!("effect-{name}"),
        program: "tool:harmless".into(),
        argv: vec!["--literal".into()],
        working_directory: SOURCE_ROOT.into(),
        environment: BTreeMap::new(),
        allowed_environment_keys: BTreeSet::new(),
        source_mount: "checkout:exact-head".into(),
        scratch_mount: Some("scratch:proof".into()),
        source_read_only: true,
        limits: limits(),
        expected: StructuredExpectation {
            stdout_sha256: hex_sha256(output),
            exit_code: 0,
        },
    }
}
fn report(request: &SandboxRequest, hits: Vec<String>) -> SandboxReport {
    SandboxReport {
        launcher: "hostile-fixture".into(),
        controls: request
            .policy
            .requirements()
            .keys()
            .map(|control| SandboxEvidence {
                control: *control,
                achieved: true,
                detail: "isolated".into(),
            })
            .collect(),
        limit_hits: hits,
        cleanup: "sandbox:fixture/process-tree-reaped".into(),
    }
}
struct FakeLauncher {
    calls: AtomicUsize,
    mode: &'static str,
}
impl SandboxLauncher for FakeLauncher {
    fn id(&self) -> &str {
        "hostile-fixture"
    }
    fn launch(&self, request: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.environment.iter().count(), 1);
        assert_eq!(request.policy.mounts()[0].access, MountAccess::ReadOnly);
        assert_eq!(
            request.policy.requirements()[&SandboxControl::Network],
            SandboxRequirement::Required
        );
        match self.mode {
            "ok" => SandboxAttempt::Completed(SandboxResult {
                stdout: b"ok".to_vec(),
                stderr: vec![],
                exit_code: 0,
                report: report(request, vec![]),
            }),
            "flood" => SandboxAttempt::Completed(SandboxResult {
                stdout: vec![b'x'; 1024],
                stderr: vec![],
                exit_code: 0,
                report: report(request, vec!["output".into()]),
            }),
            "fork" => SandboxAttempt::Stopped(report(request, vec!["process-count".into()])),
            "timeout" => SandboxAttempt::Stopped(report(request, vec!["wall-time".into()])),
            "write" => SandboxAttempt::Refused(sim_lib_exec::SandboxRefusal {
                launcher: self.id().into(),
                reason: "read-only source denied write".into(),
                report: Some(report(request, vec![])),
            }),
            "network" => SandboxAttempt::Refused(sim_lib_exec::SandboxRefusal {
                launcher: self.id().into(),
                reason: "network namespace denied connect".into(),
                report: Some(report(request, vec![])),
            }),
            _ => SandboxAttempt::Completed(SandboxResult {
                stdout: b"not structured".to_vec(),
                stderr: vec![],
                exit_code: 0,
                report: report(request, vec![]),
            }),
        }
    }
}
struct PanicLauncher;
impl SandboxLauncher for PanicLauncher {
    fn id(&self) -> &str {
        "panic"
    }
    fn launch(&self, _: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
        panic!("replay dispatched a process")
    }
}
#[derive(Default)]
struct Store(Mutex<BTreeMap<String, TypedProofReceipt>>);
impl ProofReceiptStore for Store {
    fn inspect(&self, id: &str) -> Option<TypedProofReceipt> {
        self.0.lock().unwrap().get(id).cloned()
    }
    fn record(&self, id: &str, receipt: &TypedProofReceipt) {
        self.0.lock().unwrap().insert(id.into(), receipt.clone());
    }
}
fn pins() -> ExecutionPins {
    ExecutionPins {
        conduct: "conduct".into(),
        policy: "policy".into(),
        source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [7; 32]),
        model_pick: "none".into(),
        runner_generation: "runner".into(),
    }
}

#[test]
fn catalog_rejects_every_policy_widening_and_shell_escape() {
    let mut cases = Vec::new();
    let mut shell = command("shell", b"ok");
    shell.program = "sh -c echo".into();
    cases.push(shell);
    let mut ambient = command("ambient", b"ok");
    ambient.environment.insert("HOME".into(), "/host".into());
    cases.push(ambient);
    let mut secret = command("secret", b"ok");
    secret
        .environment
        .insert("API_TOKEN".into(), "capture".into());
    secret.allowed_environment_keys.insert("API_TOKEN".into());
    cases.push(secret);
    let mut absolute = command("absolute", b"ok");
    absolute.source_mount = "/host/source".into();
    cases.push(absolute);
    let mut writable = command("writable", b"ok");
    writable.source_read_only = false;
    cases.push(writable);
    let mut cwd = command("cwd", b"ok");
    cwd.working_directory = "/etc".into();
    cases.push(cwd);
    for leaf in cases {
        assert!(ProofCatalog::new([ProofLeaf::Command(leaf)]).is_err());
    }
    let catalog = ProofCatalog::new([ProofLeaf::Command(command("known", b"ok"))]).unwrap();
    assert!(matches!(
        catalog.leaf("conduct-injected"),
        Err(ProofError::NotCatalogued(_))
    ));
}

#[test]
fn hostile_process_specimens_are_denied_or_bounded_and_exit_zero_is_not_proof() {
    for mode in ["flood", "fork", "timeout", "malformed", "write", "network"] {
        let catalog = ProofCatalog::new([ProofLeaf::Command(command(mode, b"ok"))]).unwrap();
        let launcher = FakeLauncher {
            calls: AtomicUsize::new(0),
            mode,
        };
        let receipt = execute_proof_leaf(
            &catalog,
            mode,
            &launcher,
            &ProcessCancellation::default(),
            "2026-08-25T00:00:00Z",
        )
        .unwrap();
        assert_ne!(
            receipt.disposition,
            ProofDisposition::Passed,
            "hostile fixture {mode}"
        );
        assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn pure_leaves_are_deterministic_and_never_launch() {
    let catalog = ProofCatalog::new([
        ProofLeaf::ArtifactEquality {
            name: "artifact".into(),
            left: b"same".to_vec(),
            right: b"same".to_vec(),
        },
        ProofLeaf::SourceDeckPredicate {
            name: "deck".into(),
            actual_deck: "deck:7".into(),
            expected_deck: "deck:7".into(),
            required_claims: BTreeSet::from(["claim".into()]),
            present_claims: BTreeSet::from(["claim".into()]),
        },
    ])
    .unwrap();
    for name in ["artifact", "deck"] {
        assert_eq!(
            execute_proof_leaf(
                &catalog,
                name,
                &PanicLauncher,
                &ProcessCancellation::default(),
                "fixed"
            )
            .unwrap()
            .disposition,
            ProofDisposition::Passed
        );
    }
}

#[test]
fn no_model_command_receipt_replays_with_a_launcher_that_panics_on_use() {
    let catalog = ProofCatalog::new([ProofLeaf::Command(command("no-model", b"ok"))]).unwrap();
    let journal = ExecutionJournal::new(
        Arc::new(MemoryBackend::new()),
        "proof-exec",
        Limits::default(),
    );
    let mut state = journal.open(pins(), None).unwrap();
    let store = Store::default();
    let launcher = FakeLauncher {
        calls: AtomicUsize::new(0),
        mode: "ok",
    };
    let first = execute_journaled_proof(
        &journal,
        &mut state,
        JournaledProofExecution {
            catalog: &catalog,
            name: "no-model",
            launcher: &launcher,
            receipts: &store,
            cancellation: &ProcessCancellation::default(),
            observed_at: "2026-08-25T00:00:00Z".into(),
        },
    )
    .unwrap();
    assert_eq!(first.disposition, ProofDisposition::Passed);
    assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
    let mut replayed = journal.rebuild().unwrap();
    let second = execute_journaled_proof(
        &journal,
        &mut replayed,
        JournaledProofExecution {
            catalog: &catalog,
            name: "no-model",
            launcher: &PanicLauncher,
            receipts: &store,
            cancellation: &ProcessCancellation::default(),
            observed_at: "ignored".into(),
        },
    )
    .unwrap();
    assert_eq!(second, first);
}
