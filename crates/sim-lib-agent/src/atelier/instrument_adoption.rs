//! Reproducible adoption of an instrument proposed through conversation.
//!
//! Conversation is deliberately not an input to adoption. A proposer must first
//! freeze intent as a BRIDGE brief and complete recipe. The transaction previews
//! a [`super::ChangeCapsule`], then replaces all installed identities or none.

use super::ChangeCapsule;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeInstrumentBrief {
    pub goal: String,
    pub non_goals: Vec<String>,
    pub shapes: Vec<String>,
    pub authority: Vec<String>,
    pub privacy: Vec<String>,
    pub latency: String,
    pub stop: Vec<String>,
    pub acceptance: Vec<String>,
}

impl BridgeInstrumentBrief {
    fn validate(&self) -> Result<(), AdoptionError> {
        if self.goal.trim().is_empty()
            || self.non_goals.is_empty()
            || self.shapes.is_empty()
            || self.authority.is_empty()
            || self.privacy.is_empty()
            || self.latency.trim().is_empty()
            || self.stop.is_empty()
            || self.acceptance.is_empty()
        {
            return Err(AdoptionError::IncompleteBrief);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactDigest {
    pub path: String,
    pub content_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackClosure {
    pub roots: Vec<String>,
    pub members: Vec<String>,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenInstrumentRecipe {
    pub id: String,
    pub source_content_id: String,
    pub builder: String,
    pub sandbox_route: String,
    pub hotload_route: String,
    pub pack: PackClosure,
    pub capabilities: Vec<String>,
    pub validation: Vec<String>,
    pub artifacts: Vec<ArtifactDigest>,
}

/// Reviewable proposal. Ephemeral chat and model state are intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentProposal {
    pub brief: BridgeInstrumentBrief,
    pub recipe: FrozenInstrumentRecipe,
    pub capsule: ChangeCapsule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptionPreview {
    pub source_content_id: String,
    pub artifacts: Vec<ArtifactDigest>,
    pub pack_closure_digest: String,
    pub capabilities: Vec<String>,
    pub validation: Vec<String>,
    pub rollback_generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentGeneration {
    pub generation: String,
    pub recipe_id: String,
    pub source_content_id: String,
    pub artifacts: Vec<ArtifactDigest>,
    pub pack_roots: Vec<String>,
    pub pack_closure_digest: String,
    pub registry_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackPoint {
    pub prior: InstrumentGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptionJournalReceipt {
    pub transaction_id: String,
    pub prior_generation: String,
    pub installed_generation: String,
    pub recipe_id: String,
    pub source_content_id: String,
    pub pack_closure_digest: String,
    pub registry_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptionState {
    pub installed: InstrumentGeneration,
    pub rollback: Option<RollbackPoint>,
    pub journal: Vec<AdoptionJournalReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdoptionError {
    IncompleteBrief,
    UnreviewedCapsule,
    MissingCanonicalRoute,
    InvalidPackClosure,
    ArtifactMismatch,
    ValidationMissing,
    NoRollbackPoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentAdoption {
    state: AdoptionState,
}

impl InstrumentAdoption {
    pub fn new(installed: InstrumentGeneration) -> Self {
        Self {
            state: AdoptionState {
                installed,
                rollback: None,
                journal: Vec::new(),
            },
        }
    }
    pub fn state(&self) -> &AdoptionState {
        &self.state
    }
    pub fn preview(&self, proposal: &InstrumentProposal) -> Result<AdoptionPreview, AdoptionError> {
        validate(proposal)?;
        Ok(AdoptionPreview {
            source_content_id: proposal.recipe.source_content_id.clone(),
            artifacts: proposal.recipe.artifacts.clone(),
            pack_closure_digest: proposal.recipe.pack.digest.clone(),
            capabilities: proposal.recipe.capabilities.clone(),
            validation: proposal.recipe.validation.clone(),
            rollback_generation: self.state.installed.generation.clone(),
        })
    }
    /// Cancelling is effect-free: proposal state is never retained.
    pub fn cancel(&self, proposal: &InstrumentProposal) -> Result<(), AdoptionError> {
        self.preview(proposal).map(|_| ())
    }
    pub fn adopt(
        &mut self,
        transaction_id: impl Into<String>,
        generation: impl Into<String>,
        registry_selection: impl Into<String>,
        proposal: &InstrumentProposal,
        clean_rebuild: &[ArtifactDigest],
    ) -> Result<AdoptionJournalReceipt, AdoptionError> {
        self.preview(proposal)?;
        if normalized(clean_rebuild) != normalized(&proposal.recipe.artifacts) {
            return Err(AdoptionError::ArtifactMismatch);
        }
        let prior = self.state.installed.clone();
        let installed = InstrumentGeneration {
            generation: generation.into(),
            recipe_id: proposal.recipe.id.clone(),
            source_content_id: proposal.recipe.source_content_id.clone(),
            artifacts: normalized(clean_rebuild),
            pack_roots: proposal.recipe.pack.roots.clone(),
            pack_closure_digest: proposal.recipe.pack.digest.clone(),
            registry_selection: registry_selection.into(),
        };
        let receipt = AdoptionJournalReceipt {
            transaction_id: transaction_id.into(),
            prior_generation: prior.generation.clone(),
            installed_generation: installed.generation.clone(),
            recipe_id: installed.recipe_id.clone(),
            source_content_id: installed.source_content_id.clone(),
            pack_closure_digest: installed.pack_closure_digest.clone(),
            registry_selection: installed.registry_selection.clone(),
        };
        self.state.installed = installed;
        self.state.rollback = Some(RollbackPoint { prior });
        self.state.journal.push(receipt.clone());
        Ok(receipt)
    }
    pub fn rollback(&mut self) -> Result<InstrumentGeneration, AdoptionError> {
        let point = self
            .state
            .rollback
            .take()
            .ok_or(AdoptionError::NoRollbackPoint)?;
        self.state.installed = point.prior;
        Ok(self.state.installed.clone())
    }
}

fn validate(proposal: &InstrumentProposal) -> Result<(), AdoptionError> {
    proposal.brief.validate()?;
    if proposal.capsule.patches.is_empty() || proposal.capsule.rollback_notes.is_empty() {
        return Err(AdoptionError::UnreviewedCapsule);
    }
    if proposal.recipe.builder.trim().is_empty()
        || proposal.recipe.sandbox_route.trim().is_empty()
        || proposal.recipe.hotload_route.trim().is_empty()
    {
        return Err(AdoptionError::MissingCanonicalRoute);
    }
    let members: BTreeSet<_> = proposal.recipe.pack.members.iter().collect();
    if proposal.recipe.pack.roots.is_empty()
        || proposal.recipe.pack.digest.trim().is_empty()
        || proposal
            .recipe
            .pack
            .roots
            .iter()
            .any(|root| !members.contains(root))
    {
        return Err(AdoptionError::InvalidPackClosure);
    }
    if proposal.recipe.validation.is_empty() || proposal.recipe.artifacts.is_empty() {
        return Err(AdoptionError::ValidationMissing);
    }
    Ok(())
}

fn normalized(artifacts: &[ArtifactDigest]) -> Vec<ArtifactDigest> {
    let mut artifacts = artifacts.to_vec();
    artifacts.sort();
    artifacts
}
