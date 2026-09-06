use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use sim_conformance_core::CheckerReceiptId;
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
    checker_receipts: BTreeMap<CheckerReceiptId, RetainedCheckerReceipt>,
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
        Ok(Self {
            leaves: by_name,
            checker_receipts: BTreeMap::new(),
        })
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
    #[error("checker receipt is absent from the grounded proof catalog")]
    CheckerReceiptNotCatalogued,
    #[error("invalid checker receipt: {0}")]
    InvalidCheckerReceipt(String),
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

/// Complete immutable inputs for one journaled proof execution.
pub struct JournaledProofExecution<'a> {
    /// Grounded proof catalog that owns the selected leaf.
    pub catalog: &'a ProofCatalog,
    /// Exact catalog leaf name.
    pub name: &'a str,
    /// Sandboxed effect launcher used only by command leaves.
    pub launcher: &'a dyn SandboxLauncher,
    /// Durable launcher-receipt authority used after ambiguous interruption.
    pub receipts: &'a dyn ProofReceiptStore,
    /// Cooperative cancellation token for a command leaf.
    pub cancellation: &'a ProcessCancellation,
    /// Caller-supplied observation timestamp recorded in the typed receipt.
    pub observed_at: String,
}

/// Journals intent before dispatch and the normalized receipt after dispatch.
/// An unresolved intent is reconciled from the launcher store and never launched twice.
pub fn execute_journaled_proof<B: JournalBackend>(
    journal: &ExecutionJournal<B>,
    state: &mut RebuiltExecution,
    execution: JournaledProofExecution<'_>,
) -> Result<TypedProofReceipt, ProofError> {
    let JournaledProofExecution {
        catalog,
        name,
        launcher,
        receipts,
        cancellation,
        observed_at,
    } = execution;
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

include!("proof/normalization.rs");

mod receipt;
pub use receipt::RetainedCheckerReceipt;

#[cfg(test)]
mod tests;
// conformance: typed hostile-sandbox proof leaves and replay.
