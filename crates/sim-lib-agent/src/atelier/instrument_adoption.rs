//! Reproducible adoption of an instrument proposed through conversation.
//!
//! Conversation is deliberately not an input to adoption. A proposer must first
//! freeze intent as a BRIDGE brief and complete recipe. The transaction previews
//! a [`super::ChangeCapsule`], then replaces all installed identities or none.

use super::ChangeCapsule;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Frozen BRIDGE brief that defines the proposed instrument's review boundary.
pub struct BridgeInstrumentBrief {
    /// Positive outcome the instrument must provide.
    pub goal: String,
    /// Explicit outcomes and behaviors outside the proposal.
    pub non_goals: Vec<String>,
    /// Data and operation Shapes crossing the instrument boundary.
    pub shapes: Vec<String>,
    /// Capabilities and principals authorized to exercise them.
    pub authority: Vec<String>,
    /// Privacy constraints applied to inputs, outputs, and evidence.
    pub privacy: Vec<String>,
    /// Declared latency budget or operating class.
    pub latency: String,
    /// Conditions that halt execution without partial adoption.
    pub stop: Vec<String>,
    /// Checked conditions required before adoption.
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
/// Immutable identity of one built artifact in an instrument generation.
pub struct ArtifactDigest {
    /// Stable artifact path inside the pack.
    pub path: String,
    /// Canonical content identity of the artifact bytes.
    pub content_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Exact transitive pack closure selected by a frozen recipe.
pub struct PackClosure {
    /// Requested root packages.
    pub roots: Vec<String>,
    /// Complete resolved closure, including every root.
    pub members: Vec<String>,
    /// Canonical digest of the ordered closure.
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Reproducible build, sandbox, hotload, and validation recipe.
pub struct FrozenInstrumentRecipe {
    /// Stable recipe identity.
    pub id: String,
    /// Content identity of the reviewed source input.
    pub source_content_id: String,
    /// Canonical builder route.
    pub builder: String,
    /// Canonical sandbox route used during construction and checks.
    pub sandbox_route: String,
    /// Canonical hotload route used to install the result.
    pub hotload_route: String,
    /// Exact package closure admitted by the recipe.
    pub pack: PackClosure,
    /// Capabilities requested by the installed instrument.
    pub capabilities: Vec<String>,
    /// Mandatory validation commands or proof identities.
    pub validation: Vec<String>,
    /// Expected clean-build artifact identities.
    pub artifacts: Vec<ArtifactDigest>,
}

/// Reviewable proposal. Ephemeral chat and model state are intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentProposal {
    /// Human-reviewable frozen intent.
    pub brief: BridgeInstrumentBrief,
    /// Reproducible implementation and validation recipe.
    pub recipe: FrozenInstrumentRecipe,
    /// Reviewed transactional change capsule.
    pub capsule: ChangeCapsule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Effect-free projection of the generation that adoption would install.
pub struct AdoptionPreview {
    /// Reviewed source identity.
    pub source_content_id: String,
    /// Artifacts expected from a clean rebuild.
    pub artifacts: Vec<ArtifactDigest>,
    /// Exact selected package-closure digest.
    pub pack_closure_digest: String,
    /// Capabilities the generation would request.
    pub capabilities: Vec<String>,
    /// Validation required before installation.
    pub validation: Vec<String>,
    /// Current generation retained as the rollback target.
    pub rollback_generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Complete installed identity of one instrument generation.
pub struct InstrumentGeneration {
    /// Stable generation identity.
    pub generation: String,
    /// Frozen recipe that produced the generation.
    pub recipe_id: String,
    /// Reviewed source identity used by the build.
    pub source_content_id: String,
    /// Installed artifact identities.
    pub artifacts: Vec<ArtifactDigest>,
    /// Root packages selected by the recipe.
    pub pack_roots: Vec<String>,
    /// Digest of the complete installed package closure.
    pub pack_closure_digest: String,
    /// Registry selection atomically bound to the generation.
    pub registry_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Recoverable predecessor retained by a completed adoption.
pub struct RollbackPoint {
    /// Complete generation replaced by the current install.
    pub prior: InstrumentGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Append-only evidence for one atomic instrument replacement.
pub struct AdoptionJournalReceipt {
    /// Caller-assigned transaction identity.
    pub transaction_id: String,
    /// Generation active before the transaction.
    pub prior_generation: String,
    /// Generation installed by the transaction.
    pub installed_generation: String,
    /// Frozen recipe used by the transaction.
    pub recipe_id: String,
    /// Reviewed source identity used by the transaction.
    pub source_content_id: String,
    /// Installed package-closure digest.
    pub pack_closure_digest: String,
    /// Registry selection installed with the generation.
    pub registry_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Current generation, recoverable predecessor, and adoption history.
pub struct AdoptionState {
    /// Generation currently selected by the registry.
    pub installed: InstrumentGeneration,
    /// Most recently replaced generation, when rollback remains available.
    pub rollback: Option<RollbackPoint>,
    /// Ordered receipts for completed adoption transactions.
    pub journal: Vec<AdoptionJournalReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Fail-closed reason an instrument proposal cannot advance.
pub enum AdoptionError {
    /// The frozen brief omits a mandatory review dimension.
    IncompleteBrief,
    /// The change capsule has no patch or rollback evidence.
    UnreviewedCapsule,
    /// Builder, sandbox, or hotload routing is absent.
    MissingCanonicalRoute,
    /// Package roots are absent from the resolved closure or its digest is missing.
    InvalidPackClosure,
    /// Clean rebuild output differs from the reviewed artifact set.
    ArtifactMismatch,
    /// The recipe supplies no validation or expected artifacts.
    ValidationMissing,
    /// No recoverable predecessor is available.
    NoRollbackPoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Transactional owner of instrument preview, adoption, and rollback state.
pub struct InstrumentAdoption {
    state: AdoptionState,
}

impl InstrumentAdoption {
    /// Starts adoption management from an already installed generation.
    pub fn new(installed: InstrumentGeneration) -> Self {
        Self {
            state: AdoptionState {
                installed,
                rollback: None,
                journal: Vec::new(),
            },
        }
    }
    /// Returns the complete current adoption state.
    pub fn state(&self) -> &AdoptionState {
        &self.state
    }
    /// Validates a proposal and returns its effect-free installation projection.
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
    /// Atomically installs a cleanly reproduced proposal and records its receipt.
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
    /// Restores the retained predecessor and consumes the rollback point.
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
