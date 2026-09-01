use crate::{Blocker, PlanKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionKind {
    Grounding,
    Atomicity,
    Readiness,
    Blocker,
    Completion,
    Invalidation,
    Promise,
    Impossibility,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    pub kind: DecisionKind,
    pub summary: String,
    pub path: Vec<PlanKey>,
}

pub fn blocker_explanation(blocker: &Blocker, path: Vec<PlanKey>, maximum: usize) -> Explanation {
    let mut path = path;
    path.truncate(maximum);
    Explanation {
        kind: DecisionKind::Blocker,
        summary: format!("{blocker:?}"),
        path,
    }
}
