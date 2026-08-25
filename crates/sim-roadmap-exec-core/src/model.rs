use std::{collections::BTreeSet, fmt};

use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{ContentId, Error, Expr, Result, Symbol};
use sim_roadmap_core::{PhaseId, PromiseId};

macro_rules! text_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Symbol);
        impl $name {
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                if value.is_empty() || value.len() > 128 || value.contains(char::is_whitespace) {
                    return Err(Error::Eval(format!(concat!(
                        $label,
                        " must be 1..=128 non-whitespace bytes"
                    ))));
                }
                Ok(Self(Symbol::qualified("roadmap-exec", value)))
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new("unset").expect("valid")
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl CitizenField for $name {
            fn encode_field(&self) -> Expr {
                self.0.encode_field()
            }
            fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
                let symbol = Symbol::decode_field_expr(expr, field)?;
                let text = symbol.to_string();
                let value = text
                    .strip_prefix("roadmap-exec/")
                    .ok_or_else(|| Error::Eval(format!("{field} has foreign id namespace")))?;
                Self::new(value)
            }
        }
    };
}
text_id!(ExecutionId, "execution id");
text_id!(ExecutionPolicyId, "execution policy id");
text_id!(AttemptId, "attempt id");
text_id!(MutationId, "mutation id");

/// Closed lifecycle state; extension belongs in observations, never lifecycle semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PhaseRunState {
    #[default]
    Planned,
    Running,
    Reconciling,
    Succeeded,
    Failed,
    Cancelled,
}
impl fmt::Display for PhaseRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl CitizenField for PhaseRunState {
    fn encode_field(&self) -> Expr {
        Symbol::new(match self {
            Self::Planned => "planned",
            Self::Running => "running",
            Self::Reconciling => "reconciling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        })
        .encode_field()
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        match Symbol::decode_field_expr(expr, field)?.to_string().as_str() {
            "planned" => Ok(Self::Planned),
            "running" => Ok(Self::Running),
            "reconciling" => Ok(Self::Reconciling),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(Error::Eval(format!("{field} has unknown phase state"))),
        }
    }
}

/// Immutable policy pins the identities required to judge a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionPolicy {
    pub id: ExecutionPolicyId,
    pub source_deck: ContentId,
    pub required_promises: Vec<PromiseId>,
    pub required_proofs: Vec<Symbol>,
}

/// Portable bytes-free image: adapters resolve content by identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileImage {
    pub path: String,
    pub content: Option<ContentId>,
}

/// Canonically sorted, path-unique mutation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationPlan {
    pub id: MutationId,
    pub preimages: Vec<FileImage>,
    pub postimages: Vec<FileImage>,
}
impl MutationPlan {
    pub fn new(
        id: MutationId,
        mut preimages: Vec<FileImage>,
        mut postimages: Vec<FileImage>,
    ) -> std::result::Result<Self, ExecutionFailure> {
        for images in [&mut preimages, &mut postimages] {
            images.sort();
            if images.windows(2).any(|w| w[0].path == w[1].path) {
                return Err(ExecutionFailure::DuplicatePath);
            }
            if images.iter().any(|i| !valid_path(&i.path)) {
                return Err(ExecutionFailure::InvalidPath);
            }
        }
        Ok(Self {
            id,
            preimages,
            postimages,
        })
    }
    pub fn classify(&self, image: &FileImage) -> ImageClass {
        let pre = self.preimages.iter().any(|i| i == image);
        let post = self.postimages.iter().any(|i| i == image);
        match (pre, post) {
            (true, false) => ImageClass::Preimage,
            (false, true) => ImageClass::Postimage,
            (true, true) => ImageClass::PreAndPost,
            _ => ImageClass::Foreign,
        }
    }
}
fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 4096
        && !p.starts_with('/')
        && !p.split('/').any(|s| s.is_empty() || s == "." || s == "..")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageClass {
    Preimage,
    Postimage,
    PreAndPost,
    Foreign,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofCursor {
    pub sequence: u64,
    pub journal_head: ContentId,
    pub proof: Symbol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromiseDischarge {
    pub promise: PromiseId,
    pub status: Symbol,
    pub evidence: Option<ContentId>,
}
impl PromiseDischarge {
    pub fn proven(&self) -> bool {
        self.status == Symbol::new("proven") && self.evidence.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnresolvedProof {
    pub proof: Symbol,
    pub mandatory: bool,
    pub reason: Symbol,
}

/// Canonically ordered proof obligations that still prevent completion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnresolvedReport {
    pub proofs: Vec<UnresolvedProof>,
}

impl UnresolvedReport {
    pub fn new(proofs: impl IntoIterator<Item = UnresolvedProof>) -> Self {
        Self {
            proofs: sorted_unique(proofs),
        }
    }

    pub fn has_mandatory(&self) -> bool {
        self.proofs.iter().any(|proof| proof.mandatory)
    }
}

/// Open observation category plus typed correlation and evidence fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub kind: Symbol,
    pub journal_head: ContentId,
    pub mutation: Option<MutationId>,
    pub proof_cursor: Option<ProofCursor>,
    pub image: Option<FileImage>,
    pub discharge: Option<PromiseDischarge>,
    pub unresolved: Option<UnresolvedProof>,
    pub source_deck: Option<ContentId>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            kind: Symbol::new("unspecified"),
            journal_head: placeholder_content(),
            mutation: None,
            proof_cursor: None,
            image: None,
            discharge: None,
            unresolved: None,
            source_deck: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionEvent {
    pub execution: ExecutionId,
    pub phase: PhaseId,
    pub attempt: AttemptId,
    pub observation: Observation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRequest {
    pub kind: Symbol,
    pub execution: ExecutionId,
    pub phase: PhaseId,
    pub mutation: Option<MutationId>,
    pub proof_cursor: Option<ProofCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseReceipt {
    pub execution: ExecutionId,
    pub phase: PhaseId,
    pub source_deck: ContentId,
    pub journal_head: ContentId,
    pub committed_postimages: Vec<FileImage>,
    pub discharges: Vec<PromiseDischarge>,
    pub parent_acceptance_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub preimages: Vec<FileImage>,
    pub postimages: Vec<FileImage>,
    pub foreign: Vec<FileImage>,
    pub unresolved: Vec<UnresolvedProof>,
}

impl ReconciliationReport {
    /// Classifies observations against a plan and canonicalizes every row.
    pub fn from_images(
        plan: &MutationPlan,
        images: impl IntoIterator<Item = FileImage>,
        unresolved: impl IntoIterator<Item = UnresolvedProof>,
    ) -> Self {
        let mut report = Self {
            preimages: Vec::new(),
            postimages: Vec::new(),
            foreign: Vec::new(),
            unresolved: sorted_unique(unresolved),
        };
        for image in images {
            match plan.classify(&image) {
                ImageClass::Preimage => report.preimages.push(image),
                ImageClass::Postimage => report.postimages.push(image),
                ImageClass::PreAndPost => {
                    report.preimages.push(image.clone());
                    report.postimages.push(image);
                }
                ImageClass::Foreign => report.foreign.push(image),
            }
        }
        report.preimages = sorted_unique(report.preimages);
        report.postimages = sorted_unique(report.postimages);
        report.foreign = sorted_unique(report.foreign);
        report
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub state: PhaseRunState,
    pub journal_head: ContentId,
    pub committed_postimages: Vec<FileImage>,
    pub discharges: Vec<PromiseDischarge>,
    pub parent_acceptance_retained: bool,
    pub current_source_deck: Option<ContentId>,
    pub unresolved: Vec<UnresolvedProof>,
    pub requested_effects: Vec<EffectRequest>,
    pub receipt: Option<PhaseReceipt>,
}
impl Default for Transition {
    fn default() -> Self {
        Self {
            state: PhaseRunState::Planned,
            journal_head: placeholder_content(),
            committed_postimages: vec![],
            discharges: vec![],
            parent_acceptance_retained: false,
            current_source_deck: None,
            unresolved: vec![],
            requested_effects: vec![],
            receipt: None,
        }
    }
}

/// Canonical generic Card/Shape/read-construct face for execution values.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "roadmap-exec/Value", version = 1)]
pub struct ExecutionValueFace {
    pub kind: Symbol,
    pub fields: Expr,
}
impl Default for ExecutionValueFace {
    fn default() -> Self {
        Self {
            kind: Symbol::new("transition"),
            fields: Expr::Nil,
        }
    }
}

fn placeholder_content() -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [0; 32])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionFailure {
    WrongExecution,
    WrongPhase,
    WrongAttempt,
    WrongJournalHead,
    WrongMutation,
    WrongProofCursor,
    InvalidState,
    InvalidObservation(Symbol),
    DuplicatePath,
    InvalidPath,
    ForeignImage,
    SuccessInvariant(&'static str),
}
impl fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for ExecutionFailure {}

pub(crate) fn sorted_unique<T: Ord + Clone>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
