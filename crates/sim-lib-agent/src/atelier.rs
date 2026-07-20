//! Atelier agent-facing retrieval contracts.

pub mod capsule;
mod contract_native;
pub mod guard;
pub mod mission;
pub mod mission_control;
mod mission_handoff;
mod mission_lease;
/// Ranked retrieval of Atelier code/memory hints from an indexed corpus.
pub mod radar;
pub mod self_hosting;
pub mod tools;

pub use capsule::{
    CapsuleCommit, CapsuleFacet, CapsuleJob, CapsuleJobKind, CapsuleJobOutcome, CapsulePatch,
    CapsulePush, CapsuleReview, CapsuleScope, ChangeCapsule, GeneratedArtifact, JobSite,
    PinPlanEntry, PlacedJob, fake_change_capsule, review_change_capsule,
};
pub use contract_native::{
    AtelierBackend, CONTRACT_NATIVE_SCHEMA, ContractNativeAtelierReport, ContractNativeDeckSummary,
    ContractNativeGrammarSummary, ContractNativeGuardDenial, ContractNativeProjectionSummary,
    ContractNativeRouteAttempt, contract_native_guard_denials,
    deterministic_contract_native_report,
};
pub use guard::{
    AtelierAction, GuardCapability, GuardDecision, GuardEvaluation, GuardRefusal,
    evaluate_guarded_action, guard_action,
};
pub use mission::{AgentMission, AtelierAgentRole, HumanDecisionPoint, MissionRun, MissionScope};
pub use mission_control::{MissionControlFixture, fake_mission_control_fixture};
pub use mission_handoff::{MissionHandoffReport, run_mission_handoff};
pub use mission_lease::{
    WorkspaceLease, WorkspaceLeaseConflict, WorkspaceLeaseKind, WorkspaceLeaseMode,
    detect_workspace_lease_conflicts,
};
pub use radar::{
    RadarChunk, RadarError, RadarHint, RadarIndex, RadarQuery, RadarReport, RadarResult,
    SourceSpan, retrieve_radar_hints,
};
pub use self_hosting::{
    SelfHostingScenario, cassette_content_hash, self_hosting_scenarios,
    validate_self_hosting_scenarios,
};
pub use tools::{
    AtelierToolAction, AtelierToolDescriptor, AtelierToolEvaluation, DocsOperation,
    DocsRegenerationRequest, PinUpdateRequest, ToolRunEvidence, evaluate_atelier_tool,
    repo_docs_descriptor, repo_validation_descriptor, simctl_tool_descriptors,
};

#[cfg(test)]
mod capsule_tests;

#[cfg(test)]
mod contract_native_tests;

#[cfg(test)]
mod guard_tests;

#[cfg(test)]
mod mission_tests;

#[cfg(test)]
mod mission_control_tests;

#[cfg(test)]
mod radar_tests;

#[cfg(test)]
mod self_hosting_tests;

#[cfg(test)]
mod tools_tests;
