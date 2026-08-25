use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use sim_lib_exec::{
    ArgAtom, MountAccess, ProcessCancellation, ProgramRef, SandboxAttempt, SandboxControl,
    SandboxLauncher, SandboxLimits, SandboxMount, SandboxPolicy, SandboxRequest,
    SandboxRequirement, SealedBindings,
};
use sim_lib_journal::JournalBackend;
use thiserror::Error;

use crate::{ExecutionJournal, ExecutionJournalError, ExecutionRecord, RebuiltExecution};

const SOURCE_ROOT: &str = "/source";
const SCRATCH_ROOT: &str = "/scratch";

/// A grounded phase's complete, immutable proof vocabulary.
#[derive(Clone, Debug)]
pub struct ProofCatalog {
    leaves: BTreeMap<String, ProofLeaf>,
}

impl ProofCatalog {
    pub fn new(leaves: impl IntoIterator<Item = ProofLeaf>) -> Result<Self, ProofError> {
        let mut by_name = BTreeMap::new();
        for leaf in leaves {
            leaf.validate()?;
            let name = leaf.name().to_owned();
            if by_name.insert(name.clone(), leaf).is_some() {
                return Err(ProofError::Invalid(format!("duplicate proof leaf {name}")));
            }
        }
        if by_name.is_empty() {
            return Err(ProofError::Invalid("empty proof catalog".into()));
        }
        Ok(Self { leaves: by_name })
    }

    pub fn leaf(&self, name: &str) -> Result<&ProofLeaf, ProofError> {
        self.leaves
            .get(name)
            .ok_or_else(|| ProofError::NotCatalogued(name.into()))
    }
}

#[derive(Clone, Debug)]
pub enum ProofLeaf {
    Command(CommandProof),
    ArtifactEquality {
        name: String,
        left: Vec<u8>,
        right: Vec<u8>,
    },
    SourceDeckPredicate {
        name: String,
        actual_deck: String,
        expected_deck: String,
        required_claims: BTreeSet<String>,
        present_claims: BTreeSet<String>,
    },
}

impl ProofLeaf {
    pub fn name(&self) -> &str {
        match self {
            Self::Command(v) => &v.name,
            Self::ArtifactEquality { name, .. } | Self::SourceDeckPredicate { name, .. } => name,
        }
    }

    fn validate(&self) -> Result<(), ProofError> {
        if self.name().is_empty() || self.name().contains(char::is_whitespace) {
            return Err(ProofError::Invalid(
                "proof name must be one non-empty token".into(),
            ));
        }
        match self {
            Self::Command(v) => v.validate(),
            Self::ArtifactEquality { .. } => Ok(()),
            Self::SourceDeckPredicate {
                actual_deck,
                expected_deck,
                required_claims,
                ..
            } if actual_deck.is_empty()
                || expected_deck.is_empty()
                || required_claims.is_empty() =>
            {
                Err(ProofError::Invalid(
                    "source-deck predicate is ungrounded".into(),
                ))
            }
            Self::SourceDeckPredicate { .. } => Ok(()),
        }
    }
}

/// The command form admitted from the grounded catalog. Conduct packages select a name only.
#[derive(Clone, Debug)]
pub struct CommandProof {
    pub name: String,
    pub effect_id: String,
    pub program: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub environment: BTreeMap<String, String>,
    pub allowed_environment_keys: BTreeSet<String>,
    pub source_mount: String,
    pub scratch_mount: Option<String>,
    pub source_read_only: bool,
    pub limits: SandboxLimits,
    pub expected: StructuredExpectation,
}

#[derive(Clone, Debug)]
pub struct StructuredExpectation {
    pub stdout_sha256: String,
    pub exit_code: i32,
}

impl CommandProof {
    fn validate(&self) -> Result<(), ProofError> {
        if self.effect_id.is_empty()
            || self.program.is_empty()
            || self.program.contains(char::is_whitespace)
        {
            return Err(ProofError::Invalid(
                "program must be an opaque tool id, not a shell string".into(),
            ));
        }
        if self.argv.iter().any(|v| v.contains('\0')) {
            return Err(ProofError::Invalid("argv contains NUL".into()));
        }
        if self.working_directory != SOURCE_ROOT && self.working_directory != SCRATCH_ROOT {
            return Err(ProofError::Invalid(
                "cwd is not a declared guest root".into(),
            ));
        }
        if self.source_mount.is_empty() || self.source_mount.starts_with('/') {
            return Err(ProofError::Invalid(
                "source mount must be an opaque identity".into(),
            ));
        }
        if !self.source_read_only {
            return Err(ProofError::Invalid("proof source must be read-only".into()));
        }
        if self
            .environment
            .keys()
            .any(|key| !self.allowed_environment_keys.contains(key))
        {
            return Err(ProofError::Invalid(
                "environment key is not declared".into(),
            ));
        }
        if self.environment.keys().any(|key| {
            let key = key.to_ascii_uppercase();
            key.contains("SECRET")
                || key.contains("TOKEN")
                || key.contains("PASSWORD")
                || key.contains("KEY")
        }) {
            return Err(ProofError::Invalid(
                "credential-shaped environment key".into(),
            ));
        }
        if self.expected.stdout_sha256.len() != 64
            || !self
                .expected
                .stdout_sha256
                .bytes()
                .all(|v| v.is_ascii_hexdigit())
        {
            return Err(ProofError::Invalid(
                "expected result is not a sha256 digest".into(),
            ));
        }
        Ok(())
    }

    fn sandbox_request(&self) -> Result<SandboxRequest, ProofError> {
        let mut mounts = vec![SandboxMount {
            source: self.source_mount.clone(),
            guest_path: SOURCE_ROOT.into(),
            access: MountAccess::ReadOnly,
        }];
        if let Some(source) = &self.scratch_mount {
            if source.is_empty() || source.starts_with('/') {
                return Err(ProofError::Invalid(
                    "scratch mount must be an opaque identity".into(),
                ));
            }
            mounts.push(SandboxMount {
                source: source.clone(),
                guest_path: SCRATCH_ROOT.into(),
                access: MountAccess::Writable,
            });
        }
        let requirements = [
            SandboxControl::Network,
            SandboxControl::Mounts,
            SandboxControl::Root,
            SandboxControl::Environment,
            SandboxControl::Identity,
            SandboxControl::Cpu,
            SandboxControl::Memory,
            SandboxControl::WallTime,
            SandboxControl::ProcessCount,
            SandboxControl::FileCount,
            SandboxControl::FileBytes,
            SandboxControl::Output,
            SandboxControl::Stdin,
            SandboxControl::ProcessTree,
        ]
        .into_iter()
        .map(|control| (control, SandboxRequirement::Required));
        let policy = SandboxPolicy::new(requirements, mounts, self.limits.clone())
            .map_err(|e| ProofError::Invalid(e.to_string()))?;
        let mut environment = self.environment.clone();
        environment.insert("SIM_PROOF_CWD".into(), self.working_directory.clone());
        SandboxRequest::new(
            ProgramRef::new(self.program.clone())
                .map_err(|e| ProofError::Invalid(e.to_string()))?,
            self.argv
                .iter()
                .cloned()
                .map(ArgAtom::new)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ProofError::Invalid(e.to_string()))?,
            SealedBindings::literals(environment)
                .map_err(|e| ProofError::Invalid(e.to_string()))?,
            Vec::new(),
            policy,
        )
        .map_err(|e| ProofError::Invalid(e.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofDisposition {
    Passed,
    Failed,
    Ambiguous,
}

/// Stable observation: operational completion and semantic proof remain separate facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedProofReceipt {
    pub proof: String,
    pub effect_id: Option<String>,
    pub disposition: ProofDisposition,
    pub exit_code: Option<i32>,
    pub timeout: bool,
    pub signal: Option<i32>,
    pub truncated: bool,
    pub launcher_identity: Option<String>,
    pub sandbox_identity: Option<String>,
    pub stdout_object: Option<String>,
    pub stderr_object: Option<String>,
    pub observed_at: String,
    pub semantic_detail: String,
}

#[derive(Debug, Error)]
pub enum ProofError {
    #[error("proof leaf is not in the grounded catalog: {0}")]
    NotCatalogued(String),
    #[error("invalid proof leaf: {0}")]
    Invalid(String),
    #[error(transparent)]
    Journal(#[from] ExecutionJournalError),
    #[error("an already-launched proof effect has no conclusive launcher receipt")]
    AmbiguousEffect,
}

/// Durable launcher-receipt authority used to reconcile a crash after dispatch.
pub trait ProofReceiptStore {
    fn inspect(&self, effect_id: &str) -> Option<TypedProofReceipt>;
    fn record(&self, effect_id: &str, receipt: &TypedProofReceipt);
}

/// Journals intent before dispatch and the normalized receipt after dispatch.
/// An unresolved intent is reconciled from the launcher store and never launched twice.
pub fn execute_journaled_proof<B: JournalBackend>(
    journal: &ExecutionJournal<B>,
    state: &mut RebuiltExecution,
    catalog: &ProofCatalog,
    name: &str,
    launcher: &dyn SandboxLauncher,
    receipts: &dyn ProofReceiptStore,
    cancellation: &ProcessCancellation,
    observed_at: impl Into<String>,
) -> Result<TypedProofReceipt, ProofError> {
    let leaf = catalog.leaf(name)?;
    let ProofLeaf::Command(command) = leaf else {
        return execute_proof_leaf(catalog, name, launcher, cancellation, observed_at);
    };
    let requested = state.records.iter().any(|record| {
        matches!(record, ExecutionRecord::EffectRequested { effect_id, .. } if effect_id == &command.effect_id)
    });
    let journaled_receipt = state.records.iter().any(|record| {
        matches!(record, ExecutionRecord::EffectReceipt { effect_id, .. } if effect_id == &command.effect_id)
    });
    if requested {
        let receipt = receipts
            .inspect(&command.effect_id)
            .ok_or(ProofError::AmbiguousEffect)?;
        if !journaled_receipt {
            state.head = journal.append(
                Some(&state.head),
                ExecutionRecord::EffectReceipt {
                    effect_id: command.effect_id.clone(),
                    outcome: format!("{:?}", receipt.disposition).to_ascii_lowercase(),
                    output: None,
                },
                vec![],
            )?;
            state.records.push(ExecutionRecord::EffectReceipt {
                effect_id: command.effect_id.clone(),
                outcome: format!("{:?}", receipt.disposition).to_ascii_lowercase(),
                output: None,
            });
        }
        return Ok(receipt);
    }
    state.head = journal.append(
        Some(&state.head),
        ExecutionRecord::EffectRequested {
            effect_id: command.effect_id.clone(),
            kind: "sandbox-proof".into(),
            input: None,
        },
        vec![],
    )?;
    state.records.push(ExecutionRecord::EffectRequested {
        effect_id: command.effect_id.clone(),
        kind: "sandbox-proof".into(),
        input: None,
    });
    let receipt = execute_proof_leaf(catalog, name, launcher, cancellation, observed_at)?;
    receipts.record(&command.effect_id, &receipt);
    state.head = journal.append(
        Some(&state.head),
        ExecutionRecord::EffectReceipt {
            effect_id: command.effect_id.clone(),
            outcome: format!("{:?}", receipt.disposition).to_ascii_lowercase(),
            output: None,
        },
        vec![],
    )?;
    state.records.push(ExecutionRecord::EffectReceipt {
        effect_id: command.effect_id.clone(),
        outcome: format!("{:?}", receipt.disposition).to_ascii_lowercase(),
        output: None,
    });
    Ok(receipt)
}

/// Executes exactly one catalog leaf. Pure leaves never consult the launcher.
pub fn execute_proof_leaf(
    catalog: &ProofCatalog,
    name: &str,
    launcher: &dyn SandboxLauncher,
    cancellation: &ProcessCancellation,
    observed_at: impl Into<String>,
) -> Result<TypedProofReceipt, ProofError> {
    let observed_at = observed_at.into();
    match catalog.leaf(name)? {
        ProofLeaf::ArtifactEquality { left, right, .. } => Ok(pure_receipt(
            name,
            left == right,
            observed_at,
            "byte-for-byte artifact equality",
        )),
        ProofLeaf::SourceDeckPredicate {
            actual_deck,
            expected_deck,
            required_claims,
            present_claims,
            ..
        } => {
            let passed = actual_deck == expected_deck && required_claims.is_subset(present_claims);
            Ok(pure_receipt(
                name,
                passed,
                observed_at,
                "exact deck identity and required claims",
            ))
        }
        ProofLeaf::Command(command) => {
            let request = command.sandbox_request()?;
            let attempt = launcher.launch(&request, cancellation);
            Ok(normalize(command, attempt, observed_at))
        }
    }
}

fn pure_receipt(name: &str, passed: bool, observed_at: String, detail: &str) -> TypedProofReceipt {
    TypedProofReceipt {
        proof: name.into(),
        effect_id: None,
        disposition: if passed {
            ProofDisposition::Passed
        } else {
            ProofDisposition::Failed
        },
        exit_code: None,
        timeout: false,
        signal: None,
        truncated: false,
        launcher_identity: None,
        sandbox_identity: None,
        stdout_object: None,
        stderr_object: None,
        observed_at,
        semantic_detail: detail.into(),
    }
}

fn normalize(
    command: &CommandProof,
    attempt: SandboxAttempt,
    observed_at: String,
) -> TypedProofReceipt {
    let base = |disposition, launcher, detail| TypedProofReceipt {
        proof: command.name.clone(),
        effect_id: Some(command.effect_id.clone()),
        disposition,
        exit_code: None,
        timeout: false,
        signal: None,
        truncated: false,
        launcher_identity: launcher,
        sandbox_identity: None,
        stdout_object: None,
        stderr_object: None,
        observed_at: observed_at.clone(),
        semantic_detail: detail,
    };
    match attempt {
        SandboxAttempt::Completed(result) => {
            let digest = hex_sha256(&result.stdout);
            let controls = result
                .report
                .proves_required(&command.sandbox_request().expect("validated").policy);
            let truncated = !result.report.limit_hits.is_empty();
            let passed = controls
                && !truncated
                && result.exit_code == command.expected.exit_code
                && digest == command.expected.stdout_sha256;
            TypedProofReceipt {
                proof: command.name.clone(),
                effect_id: Some(command.effect_id.clone()),
                disposition: if passed {
                    ProofDisposition::Passed
                } else {
                    ProofDisposition::Failed
                },
                exit_code: Some(result.exit_code),
                timeout: false,
                signal: (result.exit_code < 0).then_some(-result.exit_code),
                truncated,
                launcher_identity: Some(result.report.launcher.clone()),
                sandbox_identity: Some(result.report.cleanup.clone()),
                stdout_object: Some(digest),
                stderr_object: Some(hex_sha256(&result.stderr)),
                observed_at,
                semantic_detail: if passed {
                    "structured expectation matched"
                } else {
                    "operational exit was not semantic proof"
                }
                .into(),
            }
        }
        SandboxAttempt::Stopped(report) => {
            let mut receipt = base(
                ProofDisposition::Failed,
                Some(report.launcher),
                "bounded stop".into(),
            );
            receipt.timeout = true;
            receipt.sandbox_identity = Some(report.cleanup);
            receipt
        }
        SandboxAttempt::Refused(v) => base(ProofDisposition::Failed, Some(v.launcher), v.reason),
        SandboxAttempt::Unknown(v) => base(ProofDisposition::Ambiguous, Some(v.launcher), v.reason),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
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
            &catalog,
            "no-model",
            &launcher,
            &store,
            &ProcessCancellation::default(),
            "2026-08-25T00:00:00Z",
        )
        .unwrap();
        assert_eq!(first.disposition, ProofDisposition::Passed);
        assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
        let mut replayed = journal.rebuild().unwrap();
        let second = execute_journaled_proof(
            &journal,
            &mut replayed,
            &catalog,
            "no-model",
            &PanicLauncher,
            &store,
            &ProcessCancellation::default(),
            "ignored",
        )
        .unwrap();
        assert_eq!(second, first);
    }
}
// conformance: typed hostile-sandbox proof leaves and replay.
