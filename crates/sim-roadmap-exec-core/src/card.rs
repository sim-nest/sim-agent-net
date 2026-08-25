use sim_kernel::Symbol;

/// Stable, bounded browse metadata for an execution projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionCard {
    pub key: &'static str,
    pub kind: &'static str,
    pub summary: &'static str,
    pub max_face_bytes: usize,
}

/// Complete version-one execution projection vocabulary.
pub const EXECUTION_CARDS: [ExecutionCard; 10] = [
    card(
        "roadmap.exec/summary-v1",
        "summary",
        "Execution state and next legal action.",
    ),
    card(
        "roadmap.exec/phase-v1",
        "phase",
        "Selected phase and durable state.",
    ),
    card(
        "roadmap.exec/budget-v1",
        "budget",
        "Effective limits with authority provenance.",
    ),
    card(
        "roadmap.exec/effect-v1",
        "effect",
        "Requested effect and observed receipt.",
    ),
    card(
        "roadmap.exec/mutation-v1",
        "mutation",
        "Per-path planned and observed image class.",
    ),
    card(
        "roadmap.exec/proof-v1",
        "proof",
        "Proof result and redacted evidence reference.",
    ),
    card(
        "roadmap.exec/discharge-v1",
        "discharge",
        "Promise discharge state.",
    ),
    card(
        "roadmap.exec/reconciliation-v1",
        "reconciliation",
        "Ambiguity and reconciliation facts.",
    ),
    card(
        "roadmap.exec/conduct-trace-v1",
        "conduct-trace",
        "Generic agent/topology trace identities.",
    ),
    card(
        "roadmap.exec/escalation-v1",
        "escalation",
        "Bounded stop reason and safe actions.",
    ),
];

const fn card(key: &'static str, kind: &'static str, summary: &'static str) -> ExecutionCard {
    ExecutionCard {
        key,
        kind,
        summary,
        max_face_bytes: 1024,
    }
}

pub fn execution_card(kind: &Symbol) -> Option<&'static ExecutionCard> {
    EXECUTION_CARDS
        .iter()
        .find(|card| card.kind == kind.as_qualified_str())
}
