//! Loadable, fail-closed model-task protocols.
//!
//! The protocol is intentionally open: domain, role, and family are symbols carried
//! as metadata, never variants in an engine-owned enumeration.

use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

pub type ContentId = String;

fn digest(parts: impl IntoIterator<Item = impl AsRef<[u8]>>) -> ContentId {
    let mut hash = Sha256::new();
    for part in parts {
        let bytes = part.as_ref();
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    format!("sha256:{:x}", hash.finalize())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolMetadata {
    pub id: String,
    pub domain: String,
    pub role: String,
    pub family: String,
    pub revision: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputShape {
    pub id: String,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EffectRequirement {
    pub capability: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalResponse {
    pub bytes: Vec<u8>,
    pub tool_receipts: Vec<ContentId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTrial {
    pub task_revision: ContentId,
    pub public_bytes: Vec<u8>,
    pub seed: u64,
    pub output_shape: OutputShape,
    pub effects: BTreeSet<EffectRequirement>,
    pub private_oracle: Arc<[u8]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FacetObservation {
    pub facet: String,
    pub score: f64,
    pub passed: bool,
    pub reason: String,
    pub evidence_class: EvidenceClass,
    pub provenance: ContentId,
}

pub trait ModelTaskProtocol: Send + Sync {
    fn metadata(&self) -> &ProtocolMetadata;
    fn output_shape(&self) -> OutputShape;
    fn effect_requirements(&self) -> BTreeSet<EffectRequirement>;
    fn prepare(&self, task: &TaskRevision, seed: u64) -> Result<PreparedTrial, ProtocolError>;
    fn grade(&self, prepared: &PreparedTrial, response: &TerminalResponse)
    -> Vec<FacetObservation>;
}

#[derive(Default)]
pub struct ProtocolRegistry {
    protocols: BTreeMap<String, Arc<dyn ModelTaskProtocol>>,
}
impl ProtocolRegistry {
    pub fn register(&mut self, protocol: Arc<dyn ModelTaskProtocol>) -> Result<(), ProtocolError> {
        let id = protocol.metadata().id.clone();
        if self.protocols.insert(id.clone(), protocol).is_some() {
            return Err(ProtocolError::DuplicateId(id));
        }
        Ok(())
    }
    pub fn load(&self, id: &str) -> Result<Arc<dyn ModelTaskProtocol>, ProtocolError> {
        self.protocols
            .get(id)
            .cloned()
            .ok_or_else(|| ProtocolError::UnknownProtocol(id.into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureFile {
    pub relative_path: PathBuf,
    pub blob_id: ContentId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryClosure {
    pub root: PathBuf,
    pub commit: String,
    pub files: Vec<ClosureFile>,
}
impl RepositoryClosure {
    pub fn seal(
        root: impl Into<PathBuf>,
        commit: impl Into<String>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, ProtocolError> {
        let root = root.into();
        let commit = commit.into();
        let mut files = Vec::new();
        let mut seen = BTreeSet::new();
        let canonical_root = fs::canonicalize(&root).map_err(|_| ProtocolError::Repository)?;
        for path in paths {
            validate_relative(&path)?;
            let canonical = fs::canonicalize(root.join(&path))
                .map_err(|_| ProtocolError::ClosureMissing(path.clone()))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(ProtocolError::SymlinkEscape(path));
            }
            if !seen.insert(path.clone()) {
                return Err(ProtocolError::DuplicateId(path.display().to_string()));
            }
            let tracked = git(
                &root,
                &["cat-file", "-e", &format!("{commit}:{}", path.display())],
            );
            if tracked.is_err() {
                return Err(ProtocolError::ClosureNotInCommit(path));
            }
            let committed = git_bytes(&root, &["show", &format!("{commit}:{}", path.display())])?;
            let work = fs::read(root.join(&path))
                .map_err(|_| ProtocolError::ClosureMissing(path.clone()))?;
            if committed != work {
                return Err(ProtocolError::ClosureDirty(path));
            }
            files.push(ClosureFile {
                relative_path: path,
                blob_id: digest([committed]),
            });
        }
        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(Self {
            root,
            commit,
            files,
        })
    }
    pub fn identity(&self) -> ContentId {
        digest(self.files.iter().flat_map(|f| {
            [
                f.relative_path.to_string_lossy().as_bytes().to_vec(),
                f.blob_id.as_bytes().to_vec(),
            ]
        }))
    }
}
fn validate_relative(path: &Path) -> Result<(), ProtocolError> {
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        Err(ProtocolError::SymlinkEscape(path.into()))
    } else {
        Ok(())
    }
}
fn git(root: &Path, args: &[&str]) -> Result<(), ProtocolError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|_| ProtocolError::Repository)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(ProtocolError::Repository)
    }
}
fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, ProtocolError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|_| ProtocolError::Repository)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(ProtocolError::Repository)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRevision {
    pub protocol: String,
    pub visible_inputs: Vec<u8>,
    pub hidden_oracle_id: ContentId,
    pub parser_revision: String,
    pub grader_revision: String,
    pub judge_calibration: Option<JudgeCalibration>,
    pub tools: Vec<String>,
    pub toolchain: String,
    pub seed: u64,
    pub closure: Option<RepositoryClosure>,
    id: ContentId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRevisionSpec {
    pub protocol: String,
    pub visible_inputs: Vec<u8>,
    pub hidden_oracle: Vec<u8>,
    pub parser_revision: String,
    pub grader_revision: String,
    pub judge_calibration: Option<JudgeCalibration>,
    pub tools: Vec<String>,
    pub toolchain: String,
    pub seed: u64,
    pub closure: Option<RepositoryClosure>,
}
impl TaskRevision {
    pub fn seal(spec: TaskRevisionSpec) -> Self {
        let oracle = digest([&spec.hidden_oracle]);
        let closure = spec
            .closure
            .as_ref()
            .map(RepositoryClosure::identity)
            .unwrap_or_default();
        let calibration = spec
            .judge_calibration
            .as_ref()
            .map(JudgeCalibration::identity)
            .unwrap_or_default();
        let id = digest([
            spec.protocol.as_bytes(),
            &spec.visible_inputs,
            oracle.as_bytes(),
            spec.parser_revision.as_bytes(),
            spec.grader_revision.as_bytes(),
            calibration.as_bytes(),
            spec.tools.join("\0").as_bytes(),
            spec.toolchain.as_bytes(),
            &spec.seed.to_be_bytes(),
            closure.as_bytes(),
        ]);
        Self {
            protocol: spec.protocol,
            visible_inputs: spec.visible_inputs,
            hidden_oracle_id: oracle,
            parser_revision: spec.parser_revision,
            grader_revision: spec.grader_revision,
            judge_calibration: spec.judge_calibration,
            tools: spec.tools,
            toolchain: spec.toolchain,
            seed: spec.seed,
            closure: spec.closure,
            id,
        }
    }
    pub fn content_id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscreteReceipt {
    pub generator_revision: String,
    pub seed: u64,
    pub ranks: Vec<u64>,
    pub member_ids: Vec<ContentId>,
    pub receipt_id: ContentId,
}
pub fn seal_generated_pack(
    generator_revision: &str,
    seed: u64,
    members: Vec<(u64, String, Vec<u8>)>,
    forbidden_answers: &[Vec<u8>],
) -> Result<DiscreteReceipt, ProtocolError> {
    let mut last = None;
    let mut ids = BTreeSet::new();
    let mut ranks = Vec::new();
    let mut member_ids = Vec::new();
    for (rank, id, bytes) in members {
        if last.is_some_and(|v| rank <= v) {
            return Err(ProtocolError::UnstableTraversal);
        }
        last = Some(rank);
        if !ids.insert(id.clone()) {
            return Err(ProtocolError::DuplicateId(id));
        }
        if forbidden_answers
            .iter()
            .any(|a| !a.is_empty() && bytes.windows(a.len()).any(|w| w == a))
        {
            return Err(ProtocolError::AnswerLeakage);
        }
        ranks.push(rank);
        member_ids.push(digest([id.as_bytes(), &bytes]));
    }
    let receipt_id = digest([
        generator_revision.as_bytes(),
        &seed.to_be_bytes(),
        format!("{ranks:?}").as_bytes(),
        member_ids.join("\0").as_bytes(),
    ]);
    Ok(DiscreteReceipt {
        generator_revision: generator_revision.into(),
        seed,
        ranks,
        member_ids,
        receipt_id,
    })
}
pub fn require_oracle_agreement(
    left: &[ContentId],
    right: &[ContentId],
) -> Result<(), ProtocolError> {
    if left == right {
        Ok(())
    } else {
        Err(ProtocolError::OracleDisagreement)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceClass {
    Deterministic,
    CalibratedJudge,
    Withheld,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgeCalibration {
    pub revision: String,
    pub rubric: String,
    pub anchors: Vec<ContentId>,
    pub blinded: bool,
    pub disagreement_limit: u16,
    pub abstention_allowed: bool,
}
impl JudgeCalibration {
    fn identity(&self) -> ContentId {
        digest([format!("{self:?}")])
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct JudgeVote {
    pub judge_revision: String,
    pub score: f64,
    pub abstained: bool,
    pub rationale_id: ContentId,
}
pub fn calibrated_judgment(
    cal: &JudgeCalibration,
    candidate_revision: &str,
    votes: &[JudgeVote],
    deterministic_passed: bool,
) -> FacetObservation {
    let provenance = digest([
        cal.identity().as_bytes(),
        candidate_revision.as_bytes(),
        format!("{votes:?}").as_bytes(),
    ]);
    if !deterministic_passed {
        return FacetObservation {
            facet: "judge".into(),
            score: 0.0,
            passed: false,
            reason: "deterministic failure cannot be overruled".into(),
            evidence_class: EvidenceClass::Deterministic,
            provenance,
        };
    }
    let usable: Vec<_> = votes.iter().filter(|v| !v.abstained).collect();
    if usable.is_empty() || (!cal.abstention_allowed && usable.len() != votes.len()) {
        return withheld(provenance, "judge abstention");
    }
    let min = usable.iter().map(|v| v.score).fold(f64::INFINITY, f64::min);
    let max = usable
        .iter()
        .map(|v| v.score)
        .fold(f64::NEG_INFINITY, f64::max);
    if ((max - min) * 1000.0) as u16 > cal.disagreement_limit {
        return withheld(provenance, "judge disagreement");
    }
    let score = usable.iter().map(|v| v.score).sum::<f64>() / usable.len() as f64;
    FacetObservation {
        facet: "judge".into(),
        score,
        passed: score >= 0.5,
        reason: "calibrated blinded judgment".into(),
        evidence_class: EvidenceClass::CalibratedJudge,
        provenance,
    }
}
fn withheld(provenance: ContentId, reason: &str) -> FacetObservation {
    FacetObservation {
        facet: "judge".into(),
        score: 0.0,
        passed: false,
        reason: reason.into(),
        evidence_class: EvidenceClass::Withheld,
        provenance,
    }
}

include!("protocol/fake.rs");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    DuplicateId(String),
    UnknownProtocol(String),
    WrongProtocol,
    ClosureNotInCommit(PathBuf),
    ClosureMissing(PathBuf),
    ClosureDirty(PathBuf),
    SymlinkEscape(PathBuf),
    UnstableTraversal,
    AnswerLeakage,
    OracleDisagreement,
    Repository,
}
impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model task protocol rejected input: {self:?}")
    }
}
impl std::error::Error for ProtocolError {}
