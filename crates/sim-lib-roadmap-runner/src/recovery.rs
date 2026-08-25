use sim_kernel::ContentId;
use sim_roadmap_exec_core::{
    ClassifiedFailure, RecoveryPolicy, RetryContext, RetryDecision, StopReason, admit_retry,
};

use crate::ResumeDecision;

/// Effects which recovery may request after source state has been reconciled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryEffect {
    Retry(RetryDecision),
    InvokeRefiner { derived_profile: ContentId },
    Stop(RecoveryStop),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryStop {
    Ambiguous { foreign_paths: Vec<String> },
    MutationNotReconciled,
    Retry(StopReason),
    RefinementNotLowerRank,
}

/// Reconciliation is a mandatory gate before retry, proof, rollback, or a model call.
/// An ambiguous workspace can move only after an external source-state change.
pub fn plan_retry_after_reconciliation(
    reconciliation: &ResumeDecision,
    policy: &RecoveryPolicy,
    failure: &ClassifiedFailure,
    context: &RetryContext,
) -> RecoveryEffect {
    match reconciliation {
        ResumeDecision::Ambiguous { foreign_paths } => {
            RecoveryEffect::Stop(RecoveryStop::Ambiguous {
                foreign_paths: foreign_paths.clone(),
            })
        }
        ResumeDecision::Apply { .. } => RecoveryEffect::Stop(RecoveryStop::MutationNotReconciled),
        ResumeDecision::Committed => match admit_retry(policy, failure, context) {
            decision @ RetryDecision::Retry(_) => RecoveryEffect::Retry(decision),
            RetryDecision::Stop(reason) => RecoveryEffect::Stop(RecoveryStop::Retry(reason)),
        },
    }
}

/// `NeedsRefinement` is only a proposal. The runner invokes the refiner after
/// fresh grounding/profile derivation proves a strict, still-budgeted descent.
pub fn plan_refinement_after_reconciliation(
    reconciliation: &ResumeDecision,
    freshly_regrounded: bool,
    derived_profile: ContentId,
    parent_rank: u32,
    child_rank: u32,
    policy: &RecoveryPolicy,
) -> RecoveryEffect {
    match reconciliation {
        ResumeDecision::Ambiguous { foreign_paths } => {
            RecoveryEffect::Stop(RecoveryStop::Ambiguous {
                foreign_paths: foreign_paths.clone(),
            })
        }
        ResumeDecision::Apply { .. } => RecoveryEffect::Stop(RecoveryStop::MutationNotReconciled),
        ResumeDecision::Committed
            if freshly_regrounded
                && child_rank < parent_rank
                && child_rank <= policy.max_refinement_rank =>
        {
            RecoveryEffect::InvokeRefiner { derived_profile }
        }
        ResumeDecision::Committed => RecoveryEffect::Stop(RecoveryStop::RefinementNotLowerRank),
    }
}

/// Terminal recovery states are data-only and request no effects.
pub fn terminal_requests_no_effects(effect: &RecoveryEffect) -> bool {
    matches!(effect, RecoveryEffect::Stop(_))
}
// conformance: finite identity-stable roadmap recovery decisions.
