use sim_roadmap_core::{Failure, ObligationId, PhaseId};
use sim_source_deck::SourceQuery;

use crate::RankRelation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    StaleBase,
    MissingParent(PhaseId),
    StaleParent,
    StaleGrounding,
    Ungrounded(Vec<SourceQuery>),
    ParentNotLeaf,
    TooFewChildren {
        actual: usize,
        minimum: usize,
    },
    TooManyChildren {
        actual: usize,
        maximum: usize,
    },
    InvalidRationale,
    DuplicateChild(PhaseId),
    InvalidChildParent(PhaseId),
    WidenedCeiling {
        child: PhaseId,
        field: &'static str,
    },
    UngroundedGuide {
        child: PhaseId,
        query: SourceQuery,
    },
    IncompleteCoverage(ObligationId),
    InvalidCoverage(ObligationId),
    NonDescending {
        child: PhaseId,
        relation: RankRelation,
    },
    InvalidTree(Failure),
    DependencyCompilation(String),
    OutputCompilation(String),
    InvalidSuccessor(Failure),
}
