use std::collections::BTreeSet;

use sim_roadmap_core::{AdmittedPhase, PhaseBody, PhaseSpec};
use sim_source_deck::SourceQuery;

use crate::Grounding;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkProfile {
    pub unknowns: u32,
    pub mutable_owners: u32,
    pub packages: u32,
    pub change_targets: u32,
    pub promises: u32,
    pub acceptance_groups: u32,
    pub checkpoints: u32,
}

pub fn derive_profile(
    phase: &PhaseSpec,
    admitted: &AdmittedPhase,
    grounding: &Grounding,
) -> WorkProfile {
    let packages: BTreeSet<_> = phase
        .guide
        .change_targets
        .iter()
        .filter_map(|target| target.package.clone())
        .collect();
    WorkProfile {
        unknowns: count(
            phase
                .guide
                .uses
                .iter()
                .filter(|query| !grounding.resolved.contains(*query))
                .count(),
        ),
        mutable_owners: count(admitted.effective_owners.mutable.len()),
        packages: count(packages.len()),
        change_targets: count(admitted.effective_changes.targets.len()),
        promises: count(phase.guide.promises.len()),
        acceptance_groups: count(phase.acceptance.statements.len()),
        checkpoints: match &phase.body {
            PhaseBody::Leaf { checkpoints } => count(checkpoints.len()),
            PhaseBody::Composite { .. } => 0,
        },
    }
}

pub(crate) fn unresolved(phase: &PhaseSpec, grounding: &Grounding) -> Vec<SourceQuery> {
    phase
        .guide
        .uses
        .iter()
        .filter(|query| !grounding.resolved.contains(*query))
        .cloned()
        .collect()
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
