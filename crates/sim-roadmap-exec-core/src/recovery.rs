use std::collections::BTreeMap;

use sim_kernel::ContentId;

use crate::{AttemptId, ClassifiedFailure, FailureClass};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryRule {
    pub max_attempts: u32,
    pub backoff_millis: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryPolicy {
    pub retry: BTreeMap<FailureClass, RetryRule>,
    pub max_child_attempts: u32,
    pub max_refinement_rank: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryContext {
    pub attempt: AttemptId,
    pub attempts_used: u32,
    pub unresolved_effect: bool,
    pub identities_before: Vec<ContentId>,
    pub identities_now: Vec<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryReceipt {
    pub failed_attempt: AttemptId,
    pub next_attempt_number: u32,
    pub backoff_millis: u64,
    pub unchanged_identities: Vec<ContentId>,
    pub remaining_attempts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    Retry(RetryReceipt),
    Stop(StopReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    UnresolvedEffect,
    IdentitiesChanged,
    ClassNotRetryable,
    ClassNotNamed,
    AttemptsExhausted,
}

/// Pure, single-step admission. The caller owns repetition; every admitted step
/// consumes one finite counter and carries the evidence needed for journal replay.
pub fn admit_retry(
    policy: &RecoveryPolicy,
    failure: &ClassifiedFailure,
    context: &RetryContext,
) -> RetryDecision {
    if context.unresolved_effect {
        return RetryDecision::Stop(StopReason::UnresolvedEffect);
    }
    if context.identities_before != context.identities_now {
        return RetryDecision::Stop(StopReason::IdentitiesChanged);
    }
    if !failure.class.intrinsically_retry_safe() {
        return RetryDecision::Stop(StopReason::ClassNotRetryable);
    }
    let Some(rule) = policy.retry.get(&failure.class) else {
        return RetryDecision::Stop(StopReason::ClassNotNamed);
    };
    if context.attempts_used >= rule.max_attempts {
        return RetryDecision::Stop(StopReason::AttemptsExhausted);
    }
    let next = context.attempts_used + 1;
    RetryDecision::Retry(RetryReceipt {
        failed_attempt: context.attempt.clone(),
        next_attempt_number: next,
        backoff_millis: rule
            .backoff_millis
            .get(context.attempts_used as usize)
            .copied()
            .unwrap_or(0),
        unchanged_identities: context.identities_now.clone(),
        remaining_attempts: rule.max_attempts - next,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPickRecord {
    pub record_id: ContentId,
    pub primary: ContentId,
    pub compatible_fallbacks: Vec<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildAttempt {
    pub parent: AttemptId,
    pub child: AttemptId,
    pub candidate: ContentId,
    pub pick_record: ContentId,
    pub failed_evidence_retained: Vec<ContentId>,
    pub remaining_children: u32,
}

/// Complete evidence and identity for one proposed model-fallback child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFallbackAttempt {
    /// Candidate whose failure triggered fallback consideration.
    pub failed_candidate: ContentId,
    /// Compatible candidate proposed for the child attempt.
    pub fallback: ContentId,
    /// Parent attempt that retains the failed evidence.
    pub parent: AttemptId,
    /// Fresh identity assigned to the proposed child.
    pub child: AttemptId,
    /// Number of child attempts already consumed.
    pub children_used: u32,
    /// Immutable evidence retained from the failed candidate.
    pub failed_evidence: Vec<ContentId>,
}

pub fn admit_model_fallback(
    policy: &RecoveryPolicy,
    pick: &ModelPickRecord,
    attempt: ModelFallbackAttempt,
) -> Option<ChildAttempt> {
    if attempt.failed_candidate != pick.primary
        || attempt.children_used >= policy.max_child_attempts
        || !pick.compatible_fallbacks.contains(&attempt.fallback)
    {
        return None;
    }
    Some(ChildAttempt {
        parent: attempt.parent,
        child: attempt.child,
        candidate: attempt.fallback,
        pick_record: pick.record_id.clone(),
        failed_evidence_retained: attempt.failed_evidence,
        remaining_children: policy.max_child_attempts - attempt.children_used - 1,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscalationCard {
    pub verified_state: Vec<String>,
    pub safe_paths: Vec<String>,
    pub evidence_ids: Vec<ContentId>,
    pub missing_authority_or_decision: String,
    pub permitted_next_actions: Vec<String>,
}

impl EscalationCard {
    pub const MAX_ROWS: usize = 16;
    pub const MAX_TEXT: usize = 256;

    pub fn render_redacted(&self) -> String {
        fn clean(value: &str) -> String {
            let bounded: String = value.chars().take(EscalationCard::MAX_TEXT).collect();
            if bounded.contains("<packet") || bounded.contains("secret") {
                "[redacted]".into()
            } else {
                bounded
            }
        }
        fn rows(label: &str, values: &[String], out: &mut String) {
            for value in values.iter().take(EscalationCard::MAX_ROWS) {
                out.push_str(label);
                out.push_str(&clean(value));
                out.push('\n');
            }
        }
        let mut out = String::from("CARD roadmap/recovery-escalation-v1\n");
        rows("verified: ", &self.verified_state, &mut out);
        rows("safe-path: ", &self.safe_paths, &mut out);
        for id in self.evidence_ids.iter().take(Self::MAX_ROWS) {
            out.push_str(&format!("evidence: {id:?}\n"));
        }
        out.push_str(&format!(
            "missing: {}\n",
            clean(&self.missing_authority_or_decision)
        ));
        rows("permitted: ", &self.permitted_next_actions, &mut out);
        out
    }
}
