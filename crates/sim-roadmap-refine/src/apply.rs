use std::collections::{BTreeMap, BTreeSet};

use sim_roadmap_core::{ChangeId, ObligationCoverage, PhaseBody, RevisionChange, RoadmapRevision};

use crate::{
    CoverageReport, DescentCertificate, Grounding, RankRelation, RefinementProposal, Refusal,
    TractabilityPolicy, compare_profiles, derive_profile,
};

pub trait CompilationHooks {
    fn compile_dependencies(&self, revision: &RoadmapRevision) -> Result<(), String>;
    fn compile_outputs(&self, revision: &RoadmapRevision) -> Result<(), String>;
}

pub struct NoopCompilationHooks;
impl CompilationHooks for NoopCompilationHooks {
    fn compile_dependencies(&self, _: &RoadmapRevision) -> Result<(), String> {
        Ok(())
    }
    fn compile_outputs(&self, _: &RoadmapRevision) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct AppliedRefinement {
    pub successor: RoadmapRevision,
    pub certificate: DescentCertificate,
}

pub fn apply_refinement(
    base: &RoadmapRevision,
    grounding: &Grounding,
    policy: &TractabilityPolicy,
    proposal: RefinementProposal,
    hooks: &dyn CompilationHooks,
) -> Result<AppliedRefinement, Refusal> {
    if proposal.base_revision != *base.id() {
        return Err(Refusal::StaleBase);
    }
    let parent = base
        .spec
        .phases
        .get(&proposal.parent)
        .ok_or_else(|| Refusal::MissingParent(proposal.parent.clone()))?;
    let expected_parent = crate::phase_fingerprint(parent).map_err(Refusal::OutputCompilation)?;
    if proposal.expected_parent != expected_parent {
        return Err(Refusal::StaleParent);
    }
    if proposal.expected_grounding != grounding.id {
        return Err(Refusal::StaleGrounding);
    }
    let unresolved = crate::profile::unresolved(parent, grounding);
    if !unresolved.is_empty() {
        return Err(Refusal::Ungrounded(unresolved));
    }
    if !matches!(parent.body, PhaseBody::Leaf { .. }) {
        return Err(Refusal::ParentNotLeaf);
    }
    if proposal.children.len() < 2 {
        return Err(Refusal::TooFewChildren {
            actual: proposal.children.len(),
            minimum: 2,
        });
    }
    let maximum = policy
        .maximum_children
        .min(base.spec.limits.children_per_phase);
    if proposal.children.len() > maximum {
        return Err(Refusal::TooManyChildren {
            actual: proposal.children.len(),
            maximum,
        });
    }
    if proposal.rationale.trim().is_empty() {
        return Err(Refusal::InvalidRationale);
    }

    let admitted = base.spec.admit().map_err(Refusal::InvalidTree)?;
    let parent_profile = derive_profile(parent, &admitted.phases[&proposal.parent], grounding);
    let mut ids = BTreeSet::new();
    for child in &proposal.children {
        if !ids.insert(child.id.clone()) {
            return Err(Refusal::DuplicateChild(child.id.clone()));
        }
        if child.parent.as_ref() != Some(&proposal.parent) {
            return Err(Refusal::InvalidChildParent(child.id.clone()));
        }
        check_ceilings(parent, child)?;
        if let Some(query) = child
            .guide
            .uses
            .iter()
            .find(|q| !grounding.resolved.contains(*q))
        {
            return Err(Refusal::UngroundedGuide {
                child: child.id.clone(),
                query: query.clone(),
            });
        }
    }

    let coverage = coverage_markers(parent, &proposal)?;
    let mut spec = base.spec.clone();
    let parent_mut = spec
        .phases
        .get_mut(&proposal.parent)
        .expect("checked parent");
    parent_mut.body = PhaseBody::Composite {
        children: proposal.children.iter().map(|c| c.id.clone()).collect(),
    };
    parent_mut.coverage = coverage;
    for child in proposal.children {
        spec.phases.insert(child.id.clone(), child);
    }

    let successor_admitted = spec.admit().map_err(Refusal::InvalidTree)?;
    let mut profiles = BTreeMap::new();
    let mut ordering = BTreeMap::new();
    let child_ids = match &spec.phases[&proposal.parent].body {
        PhaseBody::Composite { children } => children.clone(),
        PhaseBody::Leaf { .. } => unreachable!(),
    };
    for child_id in &child_ids {
        let profile = derive_profile(
            &spec.phases[child_id],
            &successor_admitted.phases[child_id],
            grounding,
        );
        let relation = compare_profiles(&profile, &parent_profile);
        if !matches!(relation, RankRelation::Lower { .. }) {
            return Err(Refusal::NonDescending {
                child: child_id.clone(),
                relation,
            });
        }
        profiles.insert(child_id.clone(), profile);
        ordering.insert(child_id.clone(), relation);
    }
    let successor = RoadmapRevision::new(
        Some(base.id().clone()),
        spec,
        RevisionChange {
            id: ChangeId::new(format!("refine-{}", proposal.parent))
                .map_err(Refusal::InvalidSuccessor)?,
            rationale: proposal.rationale,
        },
    )
    .map_err(Refusal::InvalidSuccessor)?;
    hooks
        .compile_dependencies(&successor)
        .map_err(Refusal::DependencyCompilation)?;
    hooks
        .compile_outputs(&successor)
        .map_err(Refusal::OutputCompilation)?;
    let certificate = DescentCertificate {
        parent: parent_profile,
        children: profiles,
        ordering,
        coverage: CoverageReport {
            obligations: proposal
                .coverage
                .iter()
                .map(|(id, values)| (id.clone(), values.iter().map(|v| v.child.clone()).collect()))
                .collect(),
            complete: true,
        },
    };
    debug_assert!(certificate.verify());
    Ok(AppliedRefinement {
        successor,
        certificate,
    })
}

fn coverage_markers(
    parent: &sim_roadmap_core::PhaseSpec,
    proposal: &RefinementProposal,
) -> Result<Vec<ObligationCoverage>, Refusal> {
    let mut markers = Vec::new();
    for obligation in parent.acceptance.statements.keys() {
        let Some(contributions) = proposal.coverage.get(obligation) else {
            return Err(Refusal::IncompleteCoverage(obligation.clone()));
        };
        if contributions.is_empty() {
            return Err(Refusal::IncompleteCoverage(obligation.clone()));
        }
        for contribution in contributions {
            let Some(child) = proposal
                .children
                .iter()
                .find(|c| c.id == contribution.child)
            else {
                return Err(Refusal::InvalidCoverage(obligation.clone()));
            };
            if !child
                .acceptance
                .statements
                .contains_key(&contribution.obligation)
            {
                return Err(Refusal::InvalidCoverage(obligation.clone()));
            }
            markers.push(ObligationCoverage::Contributes {
                parent: obligation.clone(),
                phase: contribution.child.clone(),
                child: contribution.obligation.clone(),
            });
        }
    }
    if let Some(extra) = proposal
        .coverage
        .keys()
        .find(|id| !parent.acceptance.statements.contains_key(*id))
    {
        return Err(Refusal::InvalidCoverage(extra.clone()));
    }
    Ok(markers)
}

fn check_ceilings(
    parent: &sim_roadmap_core::PhaseSpec,
    child: &sim_roadmap_core::PhaseSpec,
) -> Result<(), Refusal> {
    let id = child.id.clone();
    if !child.owners.mutable.is_empty() && !child.owners.mutable.is_subset(&parent.owners.mutable) {
        return Err(Refusal::WidenedCeiling {
            child: id,
            field: "owners.mutable",
        });
    }
    let allowed: BTreeSet<_> = parent
        .owners
        .mutable
        .union(&parent.owners.read_only)
        .collect();
    if !child.owners.read_only.is_empty()
        && !child.owners.read_only.iter().all(|v| allowed.contains(v))
    {
        return Err(Refusal::WidenedCeiling {
            child: id,
            field: "owners.read_only",
        });
    }
    macro_rules! subset {
        ($field:ident, $member:ident, $name:literal) => {
            if !child.$field.$member.is_empty()
                && !child.$field.$member.is_subset(&parent.$field.$member)
            {
                return Err(Refusal::WidenedCeiling {
                    child: id,
                    field: $name,
                });
            }
        };
    }
    subset!(resources, resources, "resources");
    subset!(capabilities, capabilities, "capabilities");
    subset!(effects, effects, "effects");
    subset!(changes, targets, "change-targets");
    Ok(())
}
