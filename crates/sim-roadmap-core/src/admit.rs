use std::collections::{BTreeMap, BTreeSet};

use crate::completion::{AggregateAcceptance, ObligationDisposition};
use crate::inheritance::{acceptance_map, narrowed, narrowed_owners};
use crate::tree::Tree;
use crate::{
    AcceptanceContract, CapabilityEnvelope, ChangeEnvelope, EffectEnvelope, Failure,
    ObligationCoverage, OwnerEnvelope, PhaseBody, PhaseDependency, PhaseId, PhaseRef,
    ResourceEnvelope, RoadmapSpec,
};

/// A phase after structural admission and root-to-leaf envelope compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedPhase {
    pub authored_owners: OwnerEnvelope,
    pub effective_owners: OwnerEnvelope,
    pub authored_resources: ResourceEnvelope,
    pub effective_resources: ResourceEnvelope,
    pub authored_capabilities: CapabilityEnvelope,
    pub effective_capabilities: CapabilityEnvelope,
    pub authored_effects: EffectEnvelope,
    pub effective_effects: EffectEnvelope,
    pub authored_changes: ChangeEnvelope,
    pub effective_changes: ChangeEnvelope,
    pub acceptance: AggregateAcceptance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedRoadmap {
    pub phases: BTreeMap<PhaseId, AdmittedPhase>,
}

impl AdmittedRoadmap {
    pub fn compile(spec: &RoadmapSpec) -> Result<Self, Failure> {
        let tree = Tree::validate(spec)?;
        reject_completion_cycles(spec, &tree)?;
        let mut phases = BTreeMap::new();
        compile_phase(spec, &tree, &spec.root, None, &mut phases)?;
        Ok(Self { phases })
    }
}

fn compile_phase(
    spec: &RoadmapSpec,
    tree: &Tree,
    id: &PhaseId,
    parent: Option<(&PhaseId, &AdmittedPhase)>,
    admitted: &mut BTreeMap<PhaseId, AdmittedPhase>,
) -> Result<(), Failure> {
    let phase = &spec.phases[id];
    let path = &tree.paths[id];
    let (owners, resources, capabilities, effects, changes, inherited) =
        if let Some((parent_id, p)) = parent {
            narrowed_owners(
                &p.effective_owners.mutable,
                &p.effective_owners.read_only,
                &phase.owners.mutable,
                &phase.owners.read_only,
                parent_id,
                id,
                path,
            )?;
            let owners = OwnerEnvelope {
                mutable: patch(&p.effective_owners.mutable, &phase.owners.mutable),
                read_only: patch(&p.effective_owners.read_only, &phase.owners.read_only),
            };
            let resources = ResourceEnvelope {
                resources: narrow_patch(
                    &p.effective_resources.resources,
                    &phase.resources.resources,
                    parent_id,
                    id,
                    "resources",
                    path,
                )?,
            };
            let capabilities = CapabilityEnvelope {
                capabilities: narrow_patch(
                    &p.effective_capabilities.capabilities,
                    &phase.capabilities.capabilities,
                    parent_id,
                    id,
                    "capabilities",
                    path,
                )?,
            };
            let effects = EffectEnvelope {
                effects: narrow_patch(
                    &p.effective_effects.effects,
                    &phase.effects.effects,
                    parent_id,
                    id,
                    "effects",
                    path,
                )?,
            };
            let changes = ChangeEnvelope {
                targets: narrow_patch(
                    &p.effective_changes.targets,
                    &phase.changes.targets,
                    parent_id,
                    id,
                    "change-targets",
                    path,
                )?,
            };
            let mut inherited = p.acceptance.inherited.clone();
            inherited.push((parent_id.clone(), p.acceptance.authored.clone()));
            (owners, resources, capabilities, effects, changes, inherited)
        } else {
            (
                phase.owners.clone(),
                phase.resources.clone(),
                phase.capabilities.clone(),
                phase.effects.clone(),
                phase.changes.clone(),
                vec![],
            )
        };

    let coverage = validate_coverage(spec, id, &phase.acceptance)?;
    let current = AdmittedPhase {
        authored_owners: phase.owners.clone(),
        effective_owners: owners,
        authored_resources: phase.resources.clone(),
        effective_resources: resources,
        authored_capabilities: phase.capabilities.clone(),
        effective_capabilities: capabilities,
        authored_effects: phase.effects.clone(),
        effective_effects: effects,
        authored_changes: phase.changes.clone(),
        effective_changes: changes,
        acceptance: AggregateAcceptance {
            authored: phase.acceptance.clone(),
            inherited,
            coverage,
        },
    };
    admitted.insert(id.clone(), current.clone());
    if let PhaseBody::Composite { children } = &phase.body {
        for child in children {
            compile_phase(spec, tree, child, Some((id, &current)), admitted)?;
        }
    }
    Ok(())
}

fn patch<T: Ord + Clone>(parent: &BTreeSet<T>, authored: &BTreeSet<T>) -> BTreeSet<T> {
    if authored.is_empty() {
        parent.clone()
    } else {
        authored.clone()
    }
}

fn narrow_patch<T: Ord + ToString + Clone>(
    parent: &BTreeSet<T>,
    authored: &BTreeSet<T>,
    parent_id: &PhaseId,
    id: &PhaseId,
    field: &'static str,
    path: &crate::CausalPath,
) -> Result<BTreeSet<T>, Failure> {
    if authored.is_empty() {
        return Ok(parent.clone());
    }
    narrowed(parent, authored, parent_id, id, field, path)
}

fn validate_coverage(
    spec: &RoadmapSpec,
    parent_id: &PhaseId,
    acceptance: &AcceptanceContract,
) -> Result<BTreeMap<crate::ObligationId, ObligationDisposition>, Failure> {
    let phase = &spec.phases[parent_id];
    let PhaseBody::Composite { children } = &phase.body else {
        return Ok(BTreeMap::new());
    };
    let path = &Tree::validate(spec)?.paths[parent_id];
    let mut dispositions = BTreeMap::new();
    let child_acceptance =
        acceptance_map(children.iter().map(|id| (id, &spec.phases[id].acceptance)));
    for marker in &phase.coverage {
        let (parent, disposition) = match marker {
            ObligationCoverage::RetainedAtParent { parent } => {
                (parent, ObligationDisposition::RetainedAtParent)
            }
            ObligationCoverage::Contributes {
                parent,
                phase,
                child,
            } => {
                if !children.contains(phase)
                    || !child_acceptance[phase].statements.contains_key(child)
                {
                    return Err(Failure::Coverage {
                        rule: "invented-child-obligation",
                        phase: parent_id.clone(),
                        obligation: child.clone(),
                        path: path.clone(),
                    });
                }
                (
                    parent,
                    ObligationDisposition::Child {
                        phase: phase.clone(),
                        obligation: child.clone(),
                    },
                )
            }
        };
        if !acceptance.statements.contains_key(parent) {
            return Err(Failure::Coverage {
                rule: "unknown-parent-obligation",
                phase: parent_id.clone(),
                obligation: parent.clone(),
                path: path.clone(),
            });
        }
        if dispositions.insert(parent.clone(), disposition).is_some() {
            return Err(Failure::Coverage {
                rule: "duplicate-parent-coverage",
                phase: parent_id.clone(),
                obligation: parent.clone(),
                path: path.clone(),
            });
        }
    }
    for obligation in acceptance.statements.keys() {
        if !dispositions.contains_key(obligation) {
            return Err(Failure::Coverage {
                rule: "dropped-parent-obligation",
                phase: parent_id.clone(),
                obligation: obligation.clone(),
                path: path.clone(),
            });
        }
    }
    let traced: BTreeSet<_> = phase
        .coverage
        .iter()
        .filter_map(|c| match c {
            ObligationCoverage::Contributes { phase, child, .. } => Some((phase, child)),
            _ => None,
        })
        .collect();
    for (child_phase, contract) in &child_acceptance {
        for obligation in contract.statements.keys() {
            if !traced.contains(&(child_phase, obligation)) {
                return Err(Failure::Coverage {
                    rule: "untraced-child-obligation",
                    phase: parent_id.clone(),
                    obligation: obligation.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    Ok(dispositions)
}

fn reject_completion_cycles(spec: &RoadmapSpec, tree: &Tree) -> Result<(), Failure> {
    for phase in spec.phases.values() {
        for dependency in &phase.dependencies {
            let referenced = match dependency {
                PhaseDependency::Requires(PhaseRef::Local(id))
                | PhaseDependency::PrefersAfter(PhaseRef::Local(id)) => Some(id),
                PhaseDependency::Consumes(output) => match &output.phase {
                    PhaseRef::Local(id) => Some(id),
                    _ => None,
                },
                _ => None,
            };
            if let Some(dependency) = referenced {
                if dependency == &phase.id || tree.descendants[&phase.id].contains(dependency) {
                    return Err(Failure::CircularCompletion {
                        phase: phase.id.clone(),
                        dependency: dependency.clone(),
                        path: tree.paths[&phase.id].clone(),
                    });
                }
            }
        }
    }
    Ok(())
}
