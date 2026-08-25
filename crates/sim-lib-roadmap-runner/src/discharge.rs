use std::collections::BTreeSet;

use sim_kernel::ContentId;
use sim_roadmap_plan::{DependencyIndex, PlanKey, SourceQueryKey};

use crate::{SourceDeckReceipt, TouchedPaths};

/// Complete invalidation frontier after committed mutation observation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FreshDeckInvalidation {
    pub touched_paths: BTreeSet<String>,
    pub affected_generated_facts: BTreeSet<String>,
}

impl FreshDeckInvalidation {
    pub fn roots(&self) -> BTreeSet<String> {
        self.touched_paths
            .union(&self.affected_generated_facts)
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FreshDeckFailure {
    OldDeckStillReusable,
    MissingInvalidationRoot(String),
    WrongRepositoryHead,
    WrongDeck,
}

/// Admit a rebuilt deck only when every changed source/generated fact was an input.
pub fn admit_fresh_deck(
    old: &SourceDeckReceipt,
    fresh: &SourceDeckReceipt,
    invalidation: &FreshDeckInvalidation,
    committed_head: &str,
    expected_deck: &ContentId,
) -> Result<(), FreshDeckFailure> {
    let roots = invalidation.roots();
    if old.reusable_after(&TouchedPaths(roots.clone())) {
        return Err(FreshDeckFailure::OldDeckStillReusable);
    }
    if let Some(missing) = missing_invalidation_root(&roots, &fresh.dependencies) {
        return Err(FreshDeckFailure::MissingInvalidationRoot(missing.clone()));
    }
    if fresh.artifact.repository_head != committed_head {
        return Err(FreshDeckFailure::WrongRepositoryHead);
    }
    if &fresh.deck_id.0 != expected_deck {
        return Err(FreshDeckFailure::WrongDeck);
    }
    Ok(())
}

/// Invalidate compiled readiness through its dependency graph; no predecessor state is edited.
pub fn invalidate_readiness(
    index: &mut DependencyIndex,
    invalidation: &FreshDeckInvalidation,
) -> BTreeSet<PlanKey> {
    invalidation
        .roots()
        .into_iter()
        .flat_map(|root| index.invalidate(&PlanKey::Source(SourceQueryKey::Excerpt(root))))
        .collect()
}

fn missing_invalidation_root<'a>(
    roots: &'a BTreeSet<String>,
    dependencies: &'a BTreeSet<String>,
) -> Option<&'a String> {
    roots.difference(dependencies).next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_source_change_invalidates_even_when_index_is_unchanged() {
        let invalidation = FreshDeckInvalidation {
            touched_paths: ["src/private.rs".into()].into(),
            affected_generated_facts: BTreeSet::new(),
        };
        assert_eq!(
            missing_invalidation_root(&invalidation.roots(), &BTreeSet::new()),
            Some(&"src/private.rs".to_owned())
        );
    }

    #[test]
    fn stale_generated_output_prevents_fresh_deck_admission() {
        let invalidation = FreshDeckInvalidation {
            touched_paths: ["src/lib.rs".into()].into(),
            affected_generated_facts: ["docs/generated/repo-contract.sx".into()].into(),
        };
        let dependencies = ["src/lib.rs".into()].into();
        assert_eq!(
            missing_invalidation_root(&invalidation.roots(), &dependencies),
            Some(&"docs/generated/repo-contract.sx".to_owned())
        );
    }

    #[test]
    fn complete_fresh_deck_frontier_is_stable() {
        let invalidation = FreshDeckInvalidation {
            touched_paths: ["src/lib.rs".into()].into(),
            affected_generated_facts: ["docs/generated/repo-contract.sx".into()].into(),
        };
        let roots = invalidation.roots();
        assert_eq!(missing_invalidation_root(&roots, &roots), None);
        assert_eq!(invalidation.roots(), roots);
    }
}
