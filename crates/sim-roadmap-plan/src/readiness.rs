use crate::{Observations, aggregate_complete};
use sim_roadmap_core::{PhaseDependency, PhaseId, PhaseRef, RoadmapSpec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    RequiredPhase(PhaseId),
    RequiredOutput(PhaseId, String),
    ImportedPhase(String),
    Promise(String),
    Acceptance(String),
    MustDescend,
}

pub fn blockers(spec: &RoadmapSpec, observations: &Observations, id: &PhaseId) -> Vec<Blocker> {
    let Some(phase) = spec.phases.get(id) else {
        return vec![Blocker::RequiredPhase(id.clone())];
    };
    let mut out = Vec::new();
    for dependency in &phase.dependencies {
        match dependency {
            PhaseDependency::Requires(PhaseRef::Local(required))
                if !aggregate_complete(spec, observations, required) =>
            {
                out.push(Blocker::RequiredPhase(required.clone()))
            }
            PhaseDependency::Consumes(output) => match &output.phase {
                PhaseRef::Local(producer)
                    if !observations
                        .outputs
                        .contains_key(&(producer.clone(), output.output.clone())) =>
                {
                    out.push(Blocker::RequiredOutput(
                        producer.clone(),
                        output.output.to_string(),
                    ))
                }
                PhaseRef::Imported { import, phase, .. } => {
                    out.push(Blocker::ImportedPhase(format!("{import}:{phase}")))
                }
                _ => {}
            },
            PhaseDependency::Requires(PhaseRef::Imported { import, phase, .. }) => {
                out.push(Blocker::ImportedPhase(format!("{import}:{phase}")))
            }
            PhaseDependency::PrefersAfter(_) | PhaseDependency::Requires(_) => {}
        }
    }
    for promise in &phase.guide.promises {
        if !observations
            .promises
            .contains(&(id.clone(), promise.id().clone()))
        {
            out.push(Blocker::Promise(promise.id().to_string()));
        }
    }
    for obligation in phase.acceptance.statements.keys() {
        if !observations
            .acceptance
            .contains(&(id.clone(), obligation.clone()))
        {
            out.push(Blocker::Acceptance(obligation.to_string()));
        }
    }
    out
}
