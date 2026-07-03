use sim_kernel::Symbol;
use sim_lib_server::{FrameEnvelope, ServerFrame};

/// Role an agent plays in a multi-agent exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRole {
    /// Decomposes goals and directs other agents.
    Planner,
    /// Carries out assigned work.
    Worker,
    /// Critiques outputs.
    Critic,
    /// Judges or scores outputs.
    Judge,
    /// Verifies correctness of outputs.
    Verifier,
    /// Produces user-facing voice or narration.
    Voice,
    /// The human or external user.
    User,
    /// System-level instruction source.
    System,
    /// A tool invocation participant.
    Tool,
    /// Retrieves supporting context.
    Retriever,
    /// Records events and traces.
    Recorder,
    /// A caller-defined role tagged by symbol.
    Custom(Symbol),
}

impl AgentRole {
    /// Returns the canonical symbol naming this role.
    pub fn as_symbol(&self) -> Symbol {
        match self {
            Self::Planner => Symbol::new("planner"),
            Self::Worker => Symbol::new("worker"),
            Self::Critic => Symbol::new("critic"),
            Self::Judge => Symbol::new("judge"),
            Self::Verifier => Symbol::new("verifier"),
            Self::Voice => Symbol::new("voice"),
            Self::User => Symbol::new("user"),
            Self::System => Symbol::new("system"),
            Self::Tool => Symbol::new("tool"),
            Self::Retriever => Symbol::new("retriever"),
            Self::Recorder => Symbol::new("recorder"),
            Self::Custom(symbol) => symbol.clone(),
        }
    }
}

/// Stamps the role onto a frame envelope and increments its hop count.
pub fn stamp_envelope_role(envelope: &mut FrameEnvelope, role: &AgentRole) {
    envelope.role = Some(role.as_symbol());
    envelope.hop = envelope.hop.saturating_add(1);
}

/// Stamps the role onto a server frame's envelope.
pub fn stamp_frame_role(frame: &mut ServerFrame, role: &AgentRole) {
    stamp_envelope_role(&mut frame.envelope, role);
}
