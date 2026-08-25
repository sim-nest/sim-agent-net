use sim_kernel::ContentId;

use crate::EffectiveCeiling;

/// Complete immutable identity envelope resolved before an effect is admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionIdentity {
    pub execution: sim_roadmap_exec_core::ExecutionId,
    pub policy: sim_roadmap_exec_core::ExecutionPolicyId,
    pub roadmap: ContentId,
    pub source_deck: ContentId,
    pub conduct: ContentId,
    pub model: ContentId,
    pub launcher: ContentId,
    pub runner: ContentId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityGrant {
    pub identity: ExecutionIdentity,
    pub ceiling: EffectiveCeiling,
    /// Fresh, caller-issued authority identity. It is deliberately not part of
    /// ExecutionIdentity so resume can require freshness without changing pins.
    pub grant: ContentId,
}
