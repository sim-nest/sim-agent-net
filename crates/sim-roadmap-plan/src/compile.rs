use crate::{
    GroundFailure, PhaseGrounding, PromiseState, PromiseVerifier, check_promise, ground_phase,
};
use sim_roadmap_core::{Failure as CoreFailure, PhaseBody, PhaseId, RoadmapRevision};
use sim_roadmap_refine::{Atomicity, TractabilityPolicy, WorkProfile, compute_atomicity};
use sim_source_deck::SourceDeck;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPhase {
    pub id: PhaseId,
    pub authored_order: usize,
    pub grounding: PhaseGrounding,
    pub profile: WorkProfile,
    pub atomicity: Atomicity,
    pub atomic: bool,
    pub promises: BTreeMap<String, PromiseState>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPlan {
    pub revision: sim_roadmap_core::RoadmapRevisionId,
    pub phases: Vec<CompiledPhase>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileFailure {
    Structure(CoreFailure),
    Ground {
        phase: PhaseId,
        failure: GroundFailure,
    },
    PromiseCollision {
        phase: PhaseId,
        promise: String,
    },
    DependencyCycle(Vec<PhaseId>),
    OutputCycle(Vec<PhaseId>),
    MissingOutput {
        phase: PhaseId,
        output: String,
    },
    GraphLimit {
        actual: usize,
        maximum: usize,
    },
}

pub fn compile_plan(
    revision: &RoadmapRevision,
    deck: &SourceDeck,
    policy: &TractabilityPolicy,
    verifier: &dyn PromiseVerifier,
) -> Result<CompiledPlan, CompileFailure> {
    let admitted = revision.spec.admit().map_err(CompileFailure::Structure)?;
    if revision.spec.phases.len() > revision.spec.limits.phases {
        return Err(CompileFailure::GraphLimit {
            actual: revision.spec.phases.len(),
            maximum: revision.spec.limits.phases,
        });
    }
    validate_outputs(revision)?;
    let order = authored_order(revision);
    let mut phases = Vec::new();
    for id in order {
        let phase = &revision.spec.phases[&id];
        let grounding = ground_phase(phase, deck).map_err(|failure| CompileFailure::Ground {
            phase: id.clone(),
            failure,
        })?;
        let atomicity =
            compute_atomicity(phase, &admitted.phases[&id], &grounding.grounding, policy);
        let profile =
            sim_roadmap_refine::derive_profile(phase, &admitted.phases[&id], &grounding.grounding);
        let mut promises = BTreeMap::new();
        for promise in &phase.guide.promises {
            let state = check_promise(&id, promise, deck, verifier);
            if state == PromiseState::Collision {
                return Err(CompileFailure::PromiseCollision {
                    phase: id,
                    promise: promise.id().to_string(),
                });
            }
            promises.insert(promise.id().to_string(), state);
        }
        let atomic = matches!(atomicity, Atomicity::Atomic { .. });
        phases.push(CompiledPhase {
            authored_order: phases.len(),
            id,
            grounding,
            profile,
            atomicity,
            atomic,
            promises,
        });
    }
    Ok(CompiledPlan {
        revision: revision.id().clone(),
        phases,
    })
}

fn authored_order(revision: &RoadmapRevision) -> Vec<PhaseId> {
    fn visit(
        id: &PhaseId,
        revision: &RoadmapRevision,
        seen: &mut BTreeSet<PhaseId>,
        out: &mut Vec<PhaseId>,
    ) {
        if !seen.insert(id.clone()) {
            return;
        }
        out.push(id.clone());
        if let PhaseBody::Composite { children } = &revision.spec.phases[id].body {
            for child in children {
                visit(child, revision, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(
        &revision.spec.root,
        revision,
        &mut BTreeSet::new(),
        &mut out,
    );
    out
}
fn validate_outputs(revision: &RoadmapRevision) -> Result<(), CompileFailure> {
    for (id, phase) in &revision.spec.phases {
        for dependency in &phase.dependencies {
            if let sim_roadmap_core::PhaseDependency::Consumes(output) = dependency
                && let sim_roadmap_core::PhaseRef::Local(producer) = &output.phase
                && !revision.spec.phases[producer]
                    .outputs
                    .contains_key(&output.output)
            {
                return Err(CompileFailure::MissingOutput {
                    phase: id.clone(),
                    output: output.output.to_string(),
                });
            }
        }
    }
    Ok(())
}
