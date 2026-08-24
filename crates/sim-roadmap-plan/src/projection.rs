use crate::{CompiledPhase, Observations, blockers};
use sim_roadmap_core::{PhaseDependency, PhaseId, PhaseRef, RoadmapSpec};

/// Complete deterministic ready set. Preference changes selection order only.
pub fn ready_set(
    spec: &RoadmapSpec,
    phases: &[CompiledPhase],
    observations: &Observations,
) -> Vec<PhaseId> {
    let authored: std::collections::BTreeMap<_, _> = phases
        .iter()
        .map(|p| (p.id.clone(), p.authored_order))
        .collect();
    let mut ready: Vec<_> = phases
        .iter()
        .filter(|p| p.atomic && blockers(spec, observations, &p.id).is_empty())
        .map(|p| p.id.clone())
        .collect();
    ready.sort_by_key(|id| {
        let preference_pending = spec.phases[id].dependencies.iter().any(|d| matches!(d, PhaseDependency::PrefersAfter(PhaseRef::Local(other)) if !crate::aggregate_complete(spec, observations, other)));
        (preference_pending, authored[id], id.clone())
    });
    ready
}
