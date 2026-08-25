use sim_kernel::ContentId;
use sim_lib_journal::{JournalError, JournalHead};
use thiserror::Error;

pub const RECORD_VERSION: u16 = 1;

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
        }
    }
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
    pub head: JournalHead,
    pub total_bytes: usize,
}

#[derive(Debug)]
pub struct ReplayFailure {
    pub last_verified_head: Option<JournalHead>,
    pub error: ExecutionJournalError,
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
    #[error("existing execution pins differ; open the returned child execution")]
    ChildRequired { child_execution_id: String },
    #[error("journal is empty")]
    Empty,
}
