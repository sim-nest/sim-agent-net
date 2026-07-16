use sim_kernel::{Cx, EvalFabric, Result};

use crate::{
    CompiledIntent, IntentLibrary, IntentStatus, LiftOptions, VerifyCatalog, forge_lift_once,
    normalize_prose,
};

/// Promotion rule applied after a resolve miss lifts a fresh candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromotePolicy {
    /// Keep the structurally checked artifact as a candidate.
    KeepCandidate,
    /// Promote only after semantic probes pass.
    ///
    /// The resolver uses its verifier catalog for probe lookup, and missing or
    /// failing probes leave the artifact as a candidate.
    AutoVerifiedOnProbePass,
    /// Require an approval record before any golden artifact is created.
    RequireHumanApprovalForGolden,
}

/// Stateful FORGE resolver with an intent library index.
#[derive(Clone, Debug)]
pub struct ForgeResolver {
    library: IntentLibrary,
    lift_options: LiftOptions,
    verify_catalog: VerifyCatalog,
}

impl ForgeResolver {
    /// Builds a resolver from an existing intent library and lift options.
    pub fn new(library: IntentLibrary, lift_options: LiftOptions) -> Self {
        Self::new_with_verifiers(library, lift_options, VerifyCatalog::new())
    }

    /// Builds a resolver from an existing intent library, lift options, and
    /// semantic verifier catalog.
    pub fn new_with_verifiers(
        library: IntentLibrary,
        lift_options: LiftOptions,
        verify_catalog: VerifyCatalog,
    ) -> Self {
        Self {
            library,
            lift_options,
            verify_catalog,
        }
    }

    /// Returns the intent library backing this resolver.
    pub fn library(&self) -> &IntentLibrary {
        &self.library
    }

    /// Returns a mutable handle to the intent library backing this resolver.
    pub fn library_mut(&mut self) -> &mut IntentLibrary {
        &mut self.library
    }

    /// Returns the semantic verifier catalog backing this resolver.
    pub fn verify_catalog(&self) -> &VerifyCatalog {
        &self.verify_catalog
    }

    /// Returns a mutable semantic verifier catalog handle.
    pub fn verify_catalog_mut(&mut self) -> &mut VerifyCatalog {
        &mut self.verify_catalog
    }

    /// Resolves prose to a compiled intent, reusing a golden source hit.
    ///
    /// A golden hit returns directly and does not call the lift fabric. A miss
    /// lifts a structurally checked candidate, stores it in the named index,
    /// and leaves it in `Candidate` state until semantic verification or human
    /// approval exists.
    pub fn resolve(
        &mut self,
        cx: &mut Cx,
        target: &dyn EvalFabric,
        prose: &str,
        policy: PromotePolicy,
    ) -> Result<CompiledIntent> {
        let (_, source) = normalize_prose(prose)?;
        if let Some(intent) = self.library.golden_by_source(&source) {
            return Ok(intent.clone());
        }

        let mut lifted = forge_lift_once(cx, target, prose, &self.lift_options)?;
        self.apply_policy(cx, &mut lifted, policy)?;
        self.library.store_resolved(lifted)
    }

    fn apply_policy(
        &self,
        cx: &mut Cx,
        intent: &mut CompiledIntent,
        policy: PromotePolicy,
    ) -> Result<()> {
        intent.status = IntentStatus::Candidate;
        match policy {
            PromotePolicy::KeepCandidate | PromotePolicy::RequireHumanApprovalForGolden => {}
            PromotePolicy::AutoVerifiedOnProbePass => {
                intent.probes = self.verify_catalog.probe_ids_for(&intent.name);
                let report = self.verify_catalog.verify_intent_probes(cx, intent)?;
                if report.accepted() {
                    intent.status = IntentStatus::Verified;
                }
            }
        }
        Ok(())
    }
}

impl Default for ForgeResolver {
    fn default() -> Self {
        Self::new(IntentLibrary::new(), LiftOptions::default())
    }
}

/// Resolves prose through an empty in-memory intent library.
///
/// Callers that need compile-once reuse across calls should keep a
/// [`ForgeResolver`] or call [`forge_resolve_with_options`] with their own
/// [`IntentLibrary`].
pub fn forge_resolve(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    prose: &str,
    policy: PromotePolicy,
) -> Result<CompiledIntent> {
    ForgeResolver::default().resolve(cx, target, prose, policy)
}

/// Resolves prose through an explicit library and lift options.
pub fn forge_resolve_with_options(
    library: &mut IntentLibrary,
    lift_options: &LiftOptions,
    cx: &mut Cx,
    target: &dyn EvalFabric,
    prose: &str,
    policy: PromotePolicy,
) -> Result<CompiledIntent> {
    let mut resolver = ForgeResolver::new(std::mem::take(library), lift_options.clone());
    let intent = resolver.resolve(cx, target, prose, policy)?;
    *library = resolver.library;
    Ok(intent)
}
