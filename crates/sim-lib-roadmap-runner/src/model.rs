use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_lib_journal::{JournalError, JournalHead};
use thiserror::Error;

pub const RECORD_VERSION: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionPins {
    pub conduct: String,
    pub policy: String,
    pub source_deck: ContentId,
    pub model_pick: String,
    pub runner_generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Packet,
    Deck,
    ProcessOutput,
    FileBytes,
    EvidenceSet,
    Snapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    pub kind: ObjectKind,
    pub content: ContentId,
    pub bytes: u64,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub struct PreparedObject {
    pub reference: ObjectRef,
    pub(crate) object: sim_lib_journal::JournalObject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionRecord {
    ExecutionOpened {
        pins: ExecutionPins,
        parent: Option<String>,
    },
    StateTransition {
        from: String,
        to: String,
    },
    EffectRequested {
        effect_id: String,
        kind: String,
        input: Option<ObjectRef>,
    },
    EffectReceipt {
        effect_id: String,
        outcome: String,
        output: Option<ObjectRef>,
    },
    MutationFence {
        mutation_id: String,
        expected: String,
    },
    ProofResult {
        proof: String,
        passed: bool,
        evidence: Option<ObjectRef>,
    },
    Discharge {
        obligation: String,
    },
    Ambiguity {
        reason: String,
    },
    TerminalReceipt {
        outcome: String,
    },
    SemanticDelta {
        cause: String,
        changes: Vec<SemanticChange>,
        evidence_set: ContentId,
    },
    Snapshot {
        covers: ContentId,
        state: ContentId,
        evidence_set: ContentId,
    },
}

impl ExecutionRecord {
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            Self::ExecutionOpened { .. } => "execution-opened",
            Self::StateTransition { .. } => "state-transition",
            Self::EffectRequested { .. } => "effect-requested",
            Self::EffectReceipt { .. } => "effect-receipt",
            Self::MutationFence { .. } => "mutation-fence",
            Self::ProofResult { .. } => "proof-result",
            Self::Discharge { .. } => "discharge",
            Self::Ambiguity { .. } => "ambiguity",
            Self::TerminalReceipt { .. } => "terminal-receipt",
            Self::SemanticDelta { .. } => "semantic-delta",
            Self::Snapshot { .. } => "snapshot",
        }
    }
}

/// One compare-and-set fact change caused by a durable execution event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticChange {
    pub key: String,
    pub before: Option<ContentId>,
    pub after: Option<ContentId>,
}

/// Reducer state reconstructed solely from cause records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CausalState {
    pub facts: BTreeMap<String, ContentId>,
    pub evidence: BTreeSet<ContentId>,
}

/// One snapshot whose semantic value was checked against its covered prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSnapshot {
    pub covers: ContentId,
    pub state: ContentId,
    pub evidence_set: ContentId,
}

/// Content that must remain reachable for exact replay and snapshot recovery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionRoots {
    pub entries: BTreeSet<ContentId>,
    pub objects: BTreeSet<ContentId>,
}

/// Result of cause-only append admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaAppend {
    /// A semantic change was appended at this head.
    Appended(JournalHead),
    /// The requested revision changed no meaning and emitted no record.
    Unchanged(JournalHead),
}

/// Optional operational signal. Telemetry is deliberately outside journal truth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub name: String,
    pub value: u64,
}

/// Effect port for optional telemetry streams.
pub trait TelemetrySink {
    fn emit(&mut self, event: TelemetryEvent);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_record_bytes: usize,
    pub max_object_bytes: usize,
    pub max_stream_bytes: usize,
    pub max_execution_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_record_bytes: 64 * 1024,
            max_object_bytes: 8 * 1024 * 1024,
            max_stream_bytes: 32 * 1024 * 1024,
            max_execution_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebuiltExecution {
    pub execution_id: String,
    pub pins: ExecutionPins,
    pub records: Vec<ExecutionRecord>,
    pub opening_head: JournalHead,
    pub head: JournalHead,
    pub total_bytes: usize,
    pub causal: CausalState,
    pub snapshots: Vec<VerifiedSnapshot>,
    pub retention: RetentionRoots,
}

#[derive(Debug)]
pub struct ReplayFailure {
    pub last_verified_head: Option<JournalHead>,
    pub error: Box<ExecutionJournalError>,
}

#[derive(Debug, Error)]
pub enum ExecutionJournalError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("execution record codec rejected input: {0}")]
    Codec(&'static str),
    #[error("execution byte budget exceeded: {0}")]
    Budget(&'static str),
    #[error("secret-shaped data was rejected before object admission")]
    Secret,
    #[error("execution identity does not match journal genesis")]
    ExecutionIdentity,
    #[error("illegal execution record at sequence {sequence}: {reason}")]
    Illegal { sequence: u64, reason: &'static str },
    #[error("object referenced by the execution record is unavailable")]
    MissingObject,
    #[error("execution semantic object is invalid: {0}")]
    Semantic(&'static str),
    #[error("existing execution pins differ; open the returned child execution")]
    ChildRequired { child_execution_id: String },
    #[error("journal is empty")]
    Empty,
}
