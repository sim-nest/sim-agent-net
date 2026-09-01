use crate::Observations;
use sim_roadmap_core::{PhaseBody, PhaseId, RoadmapSpec};

/// Aggregate completion recursively requires every child, never merely its parent marker.
pub fn aggregate_complete(spec: &RoadmapSpec, observations: &Observations, id: &PhaseId) -> bool {
    let Some(phase) = spec.phases.get(id) else {
        return false;
    };
    match &phase.body {
        PhaseBody::Leaf { .. } => observations.completed_phases.contains(id),
        PhaseBody::Composite { children } => children
            .iter()
            .all(|child| aggregate_complete(spec, observations, child)),
    }
}
