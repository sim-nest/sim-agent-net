use std::collections::BTreeMap;

use sim_roadmap_core::{ObligationId, PhaseId};

use crate::{RankRelation, WorkProfile};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageReport {
    pub obligations: BTreeMap<ObligationId, Vec<PhaseId>>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescentCertificate {
    pub parent: WorkProfile,
    pub children: BTreeMap<PhaseId, WorkProfile>,
    pub ordering: BTreeMap<PhaseId, RankRelation>,
    pub coverage: CoverageReport,
}

impl DescentCertificate {
    pub fn verify(&self) -> bool {
        self.coverage.complete
            && self.children.len() >= 2
            && self.ordering.len() == self.children.len()
            && self
                .children
                .keys()
                .all(|id| matches!(self.ordering.get(id), Some(RankRelation::Lower { .. })))
    }
}
