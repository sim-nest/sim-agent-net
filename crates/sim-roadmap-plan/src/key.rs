use sim_roadmap_core::{ObligationId, OutputId, PhaseId, PromiseId};
use sim_source_deck::SourceQuery;

/// Exact facts and derived queries in a compiled plan.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlanKey {
    Source(SourceQueryKey),
    Policy,
    Phase(PhaseId),
    Aggregate(PhaseId),
    Output(PhaseId, OutputId),
    Promise(PhaseId, PromiseId),
    Acceptance(PhaseId, ObligationId),
}

/// Orderable form of an exact source query.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceQueryKey {
    Anchor(String),
    Excerpt(String),
    Specimen(String),
}

impl From<&SourceQuery> for SourceQueryKey {
    fn from(value: &SourceQuery) -> Self {
        match value {
            SourceQuery::Anchor(v) => Self::Anchor(v.clone()),
            SourceQuery::Excerpt(v) => Self::Excerpt(v.clone()),
            SourceQuery::Specimen(v) => Self::Specimen(v.clone()),
        }
    }
}
