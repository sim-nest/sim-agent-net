use sim_kernel::{Cx, EvalFabric, Result};

use crate::{
    CompiledIntent, IntentLibrary, IntentStatus, LiftOptions, forge_lift_once, normalize_prose,
};

/// Promotion rule applied after a resolve miss lifts a fresh candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromotePolicy {
    /// Keep the structurally checked artifact as a candidate.
    KeepCandidate,
    /// Promote only after semantic probes pass.
    ///
    /// FORGE has no probe runner until the semantic-verification phase, so this
    /// policy currently keeps the artifact as a candidate.
    AutoVerifiedOnProbePass,
    /// Require an approval record before any golden artifact is created.
    RequireHumanApprovalForGolden,
}

/// Stateful FORGE resolver with an intent library index.
#[derive(Clone, Debug)]
pub struct ForgeResolver {
    library: IntentLibrary,
    lift_options: LiftOptions,
}

impl ForgeResolver {
    /// Builds a resolver from an existing intent library and lift options.
    pub fn new(library: IntentLibrary, lift_options: LiftOptions) -> Self {
        Self {
            library,
            lift_options,
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
        lifted.status = status_after_policy(policy);
        self.library.store_resolved(lifted)
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

fn status_after_policy(_policy: PromotePolicy) -> IntentStatus {
    IntentStatus::Candidate
}
