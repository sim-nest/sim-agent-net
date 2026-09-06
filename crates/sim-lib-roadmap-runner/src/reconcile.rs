use std::collections::BTreeMap;

use crate::{MutationError, PortableImage, SealedMutationPlan};

/// Durable-state classification for one path in a sealed transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathState {
    Preimage,
    Postimage,
    Unchanged,
    Foreign,
}

/// A pure recovery decision. Foreign bytes are always reported and never changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeDecision {
    Committed,
    Apply { paths: Vec<String> },
    Ambiguous { foreign_paths: Vec<String> },
}

pub fn classify_plan(
    plan: &SealedMutationPlan,
    observed: &BTreeMap<String, PortableImage>,
) -> ResumeDecision {
    let mut apply = Vec::new();
    let mut foreign = Vec::new();
    for entry in &plan.entries {
        let Some(actual) = observed.get(&entry.path) else {
            foreign.push(entry.path.clone());
            continue;
        };
        match classify_image(&entry.preimage, &entry.postimage, actual) {
            PathState::Preimage => apply.push(entry.path.clone()),
            PathState::Postimage | PathState::Unchanged => {}
            PathState::Foreign => foreign.push(entry.path.clone()),
        }
    }
    if !foreign.is_empty() {
        ResumeDecision::Ambiguous {
            foreign_paths: foreign,
        }
    } else if apply.is_empty() {
        ResumeDecision::Committed
    } else {
        ResumeDecision::Apply { paths: apply }
    }
}

pub fn classify_image(
    pre: &PortableImage,
    post: &PortableImage,
    actual: &PortableImage,
) -> PathState {
    let convert = |image: &PortableImage| sim_artifact_facet::PortableImage {
        bytes: image.bytes.clone(),
        mode: image.mode,
    };
    match sim_artifact_facet::classify_transition(&convert(pre), &convert(post), &convert(actual)) {
        Ok(sim_artifact_facet::TransitionState::Unchanged) => PathState::Unchanged,
        Ok(sim_artifact_facet::TransitionState::Base) => PathState::Preimage,
        Ok(sim_artifact_facet::TransitionState::Intended) => PathState::Postimage,
        Ok(sim_artifact_facet::TransitionState::Foreign) | Err(_) => PathState::Foreign,
    }
}

/// Construct rollback as another explicit sealed transaction. It is admitted only
/// from the exact committed postimages, never as a force operation.
pub fn inverse_plan(
    plan: &SealedMutationPlan,
    observed: &BTreeMap<String, PortableImage>,
) -> Result<SealedMutationPlan, MutationError> {
    let foreign_paths: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| observed.get(&e.path) != Some(&e.postimage))
        .map(|e| e.path.clone())
        .collect();
    if !foreign_paths.is_empty() {
        return Err(MutationError::Ambiguous { foreign_paths });
    }
    SealedMutationPlan::from_images(
        plan.entries
            .iter()
            .map(|e| (e.path.clone(), e.postimage.clone(), e.preimage.clone()))
            .collect(),
    )
}
