use std::collections::BTreeMap;

use crate::{AcceptanceContract, ObligationId, PhaseId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObligationDisposition {
    Child {
        phase: PhaseId,
        obligation: ObligationId,
    },
    RetainedAtParent,
}

/// Acceptance layers are retained verbatim instead of flattened or rewritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateAcceptance {
    pub authored: AcceptanceContract,
    pub inherited: Vec<(PhaseId, AcceptanceContract)>,
    pub coverage: BTreeMap<ObligationId, ObligationDisposition>,
}
