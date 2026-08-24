use sim_roadmap_core::{AdmittedPhase, PhaseSpec};
use sim_source_deck::SourceQuery;

use crate::{Grounding, PolicyBreach, TractabilityPolicy, WorkProfile, derive_profile};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atomicity {
    Atomic {
        profile: WorkProfile,
    },
    MustDescend {
        profile: WorkProfile,
        breaches: Vec<PolicyBreach>,
    },
    Ungrounded {
        unresolved: Vec<SourceQuery>,
    },
}

pub fn compute_atomicity(
    phase: &PhaseSpec,
    admitted: &AdmittedPhase,
    grounding: &Grounding,
    policy: &TractabilityPolicy,
) -> Atomicity {
    let unresolved = crate::profile::unresolved(phase, grounding);
    if !unresolved.is_empty() {
        return Atomicity::Ungrounded { unresolved };
    }
    let profile = derive_profile(phase, admitted, grounding);
    let breaches = policy.breaches(&profile);
    if breaches.is_empty() {
        Atomicity::Atomic { profile }
    } else {
        Atomicity::MustDescend { profile, breaches }
    }
}
