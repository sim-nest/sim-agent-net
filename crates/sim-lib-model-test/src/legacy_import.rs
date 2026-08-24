//! One-way import of sealed legacy model-test evidence.
//!
//! The importer deliberately accepts bytes, not a legacy database, executable,
//! package, transport, or cohort configuration. Hosts resolve paths from the
//! private cutover manifest, read immutable files, and pass their expected
//! digests here. This keeps the old system evidence-only after cutover.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Legacy document schemas whose evidence semantics are understood exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyKind {
    Manifest,
    Lifecycle,
    Result,
    TaskRevision,
    RequestRevision,
    DepthEpoch,
    SimRow,
    CostRecord,
}

impl LegacyKind {
    fn schema(self) -> &'static str {
        match self {
            Self::Manifest => "codebench.sweep-manifest.v1",
            Self::Lifecycle => "codebench.sweep-lifecycle.v1",
            Self::Result => "codebench.result.v1",
            Self::TaskRevision => "codebench.catalog-task-revision.v1",
            Self::RequestRevision => "codebench.catalog-request-contract.v1",
            Self::DepthEpoch => "codebench.depth-epoch.v1",
            Self::SimRow => "codebench.sim-result.v2",
            Self::CostRecord => "codebench.sweep-cost.v1",
        }
    }
}

/// Trust assigned without discarding the source object.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportClass {
    Verified,
    Partial,
    Quarantined,
}

/// Stable machine-readable classification explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportReason {
    SealedAndComplete,
    MissingDecisionEvidence,
    ReportOnlyCompatibilityIntent,
    UnknownSchema,
    DigestMismatch,
    LiveSource,
    MissingIdentity,
    ConflictingObservation,
}

/// An immutable input supplied by a host after resolving the private manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedLegacyObject {
    pub source_id: String,
    pub path: String,
    pub expected_digest: String,
    pub bytes: Vec<u8>,
    /// Pinned public Git object containing historical task/harness material.
    pub historical_git_object: Option<String>,
    /// False for files that could still be appended or rewritten.
    pub sealed: bool,
}

/// Reconstructed study coordinates. Samples deliberately do not collapse.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LegacyIdentity {
    pub subject: String,
    pub task: String,
    pub harness: String,
    pub request: String,
    pub treatment: String,
    pub sample: String,
}

/// Imported object retained for audit and deterministic projections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedLegacyObject {
    pub source_id: String,
    pub path: String,
    pub content_id: String,
    pub kind: Option<LegacyKind>,
    pub class: ImportClass,
    pub reason: ImportReason,
    pub identity: Option<LegacyIdentity>,
    pub family: Option<String>,
    pub state: Option<String>,
    pub excluded: bool,
    pub resource_units: u64,
}

impl ImportedLegacyObject {
    /// Only fully verified evidence can suppress a new trial or affect decisions.
    pub fn decision_eligible(&self) -> bool {
        self.class == ImportClass::Verified
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LegacyImportError {
    InvalidJson(String),
    DuplicateSource(String),
    UnknownSchema(String),
    DigestMismatch(String),
    LiveSource(String),
    ConflictingObservation(String),
}

impl fmt::Display for LegacyImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "legacy import rejected: {self:?}")
    }
}
impl std::error::Error for LegacyImportError {}

/// Minimal `.01` publication sink: validate the complete batch, then commit it.
pub trait LegacyBatchAdmission {
    fn existing(&self, content_id: &str) -> Option<&ImportedLegacyObject>;
    fn commit_batch(&mut self, entries: Vec<ImportedLegacyObject>);
}

#[derive(Default)]
pub struct MemoryLegacyStore {
    entries: BTreeMap<String, ImportedLegacyObject>,
}

impl MemoryLegacyStore {
    pub fn entries(&self) -> impl Iterator<Item = &ImportedLegacyObject> {
        self.entries.values()
    }
}

impl LegacyBatchAdmission for MemoryLegacyStore {
    fn existing(&self, id: &str) -> Option<&ImportedLegacyObject> {
        self.entries.get(id)
    }
    fn commit_batch(&mut self, entries: Vec<ImportedLegacyObject>) {
        for entry in entries {
            self.entries
                .entry(entry.content_id.clone())
                .or_insert(entry);
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    schema_id: String,
    schema_version: u64,
    #[serde(default)]
    subject_id: String,
    #[serde(default)]
    task_id: String,
    #[serde(default)]
    harness_id: String,
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    treatment_id: String,
    #[serde(default)]
    sample_id: String,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    excluded: bool,
    #[serde(default)]
    resource_units: u64,
    #[serde(default)]
    report_only: bool,
}

/// Atomically imports a deterministically ordered batch.
///
/// Any tampering, unknown/live source, duplicate source id, or conflicting
/// observation rejects the whole batch before the sink is changed. Exact
/// repeats are idempotent; distinct sample ids remain distinct observations.
pub fn import_legacy_batch(
    sink: &mut impl LegacyBatchAdmission,
    mut sources: Vec<SealedLegacyObject>,
) -> Result<Vec<ImportedLegacyObject>, LegacyImportError> {
    sources.sort_by(|a, b| (&a.path, &a.source_id).cmp(&(&b.path, &b.source_id)));
    let mut source_ids = BTreeSet::new();
    let mut pending = Vec::new();
    let mut coordinates: BTreeMap<LegacyIdentity, String> = BTreeMap::new();
    for source in sources {
        if !source_ids.insert(source.source_id.clone()) {
            return Err(LegacyImportError::DuplicateSource(source.source_id));
        }
        if !source.sealed {
            return Err(LegacyImportError::LiveSource(source.source_id));
        }
        let actual = digest(&source.bytes);
        if actual != source.expected_digest {
            return Err(LegacyImportError::DigestMismatch(source.source_id));
        }
        let doc: Envelope = serde_json::from_slice(&source.bytes)
            .map_err(|_| LegacyImportError::InvalidJson(source.source_id.clone()))?;
        let kind = all_kinds()
            .into_iter()
            .find(|k| k.schema() == doc.schema_id);
        let Some(kind) = kind.filter(|_| doc.schema_version == 1 || doc.schema_id.ends_with(".v2"))
        else {
            return Err(LegacyImportError::UnknownSchema(source.source_id));
        };
        let identity = reconstruct_identity(&doc, source.historical_git_object.as_deref());
        let compatibility = matches!(
            kind,
            LegacyKind::TaskRevision | LegacyKind::RequestRevision | LegacyKind::SimRow
        );
        let (class, reason) = if identity.is_none() {
            (ImportClass::Quarantined, ImportReason::MissingIdentity)
        } else if compatibility {
            (
                ImportClass::Partial,
                ImportReason::ReportOnlyCompatibilityIntent,
            )
        } else if doc.report_only {
            (ImportClass::Partial, ImportReason::MissingDecisionEvidence)
        } else {
            (ImportClass::Verified, ImportReason::SealedAndComplete)
        };
        let entry = ImportedLegacyObject {
            source_id: source.source_id,
            path: source.path,
            content_id: actual,
            kind: Some(kind),
            class,
            reason,
            identity: identity.clone(),
            family: doc.family,
            state: doc.state,
            excluded: doc.excluded,
            resource_units: doc.resource_units,
        };
        if let Some(identity) = identity
            && let Some(prior) = coordinates.insert(identity, entry.content_id.clone())
            && prior != entry.content_id
        {
            return Err(LegacyImportError::ConflictingObservation(entry.source_id));
        }
        if let Some(existing) = sink.existing(&entry.content_id) {
            if existing != &entry {
                return Err(LegacyImportError::ConflictingObservation(entry.source_id));
            }
        } else {
            pending.push(entry);
        }
    }
    sink.commit_batch(pending.clone());
    Ok(pending)
}

fn reconstruct_identity(doc: &Envelope, git: Option<&str>) -> Option<LegacyIdentity> {
    let valid_git = git.is_some_and(|v| v.len() == 40 && v.bytes().all(|b| b.is_ascii_hexdigit()));
    if !valid_git
        || [
            &doc.subject_id,
            &doc.task_id,
            &doc.harness_id,
            &doc.request_id,
            &doc.treatment_id,
            &doc.sample_id,
        ]
        .iter()
        .any(|v| v.is_empty())
    {
        return None;
    }
    Some(LegacyIdentity {
        subject: doc.subject_id.clone(),
        task: doc.task_id.clone(),
        harness: doc.harness_id.clone(),
        request: doc.request_id.clone(),
        treatment: doc.treatment_id.clone(),
        sample: doc.sample_id.clone(),
    })
}

fn all_kinds() -> [LegacyKind; 8] {
    [
        LegacyKind::Manifest,
        LegacyKind::Lifecycle,
        LegacyKind::Result,
        LegacyKind::TaskRevision,
        LegacyKind::RequestRevision,
        LegacyKind::DepthEpoch,
        LegacyKind::SimRow,
        LegacyKind::CostRecord,
    ]
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyImportReport {
    pub verified: usize,
    pub partial: usize,
    pub quarantined: usize,
    pub excluded: usize,
    pub resources: u64,
    pub by_family: BTreeMap<String, usize>,
    pub by_state: BTreeMap<String, usize>,
}

/// Rebuilds legacy-compatible totals solely from verified imported evidence.
pub fn legacy_import_report<'a>(
    entries: impl IntoIterator<Item = &'a ImportedLegacyObject>,
) -> LegacyImportReport {
    let mut out = LegacyImportReport::default();
    for entry in entries {
        match entry.class {
            ImportClass::Verified => out.verified += 1,
            ImportClass::Partial => out.partial += 1,
            ImportClass::Quarantined => out.quarantined += 1,
        }
        if !entry.decision_eligible() {
            continue;
        }
        out.excluded += usize::from(entry.excluded);
        out.resources += entry.resource_units;
        *out.by_family
            .entry(entry.family.clone().unwrap_or_else(|| "unknown".into()))
            .or_default() += 1;
        *out.by_state
            .entry(entry.state.clone().unwrap_or_else(|| "unknown".into()))
            .or_default() += 1;
    }
    out
}

/// Import parity and independently produced SIM execution parity never merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityParity {
    pub import_accounted: usize,
    pub import_expected: usize,
    pub sim_execution_matched: usize,
    pub sim_execution_expected: usize,
}
