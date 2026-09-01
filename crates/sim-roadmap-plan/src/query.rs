use sim_roadmap_core::{ObligationId, OutputId, PhaseId, PromiseId};
use std::collections::{BTreeMap, BTreeSet};

/// Current externally verified observations. Absence means not established.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observations {
    pub completed_phases: BTreeSet<PhaseId>,
    pub outputs: BTreeMap<(PhaseId, OutputId), String>,
    pub promises: BTreeSet<(PhaseId, PromiseId)>,
    pub acceptance: BTreeSet<(PhaseId, ObligationId)>,
}
