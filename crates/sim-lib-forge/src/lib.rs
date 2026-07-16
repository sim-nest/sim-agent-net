//! Compiled intent records, BRIDGE lifts, and reusable packet-program lookup.
//!
//! `sim-lib-forge` carries the stable data records that name a prose intent,
//! the normalized source content, the compiled BRIDGE packet content, and the
//! verification state that controls reuse. Its lift paths ask a model for a
//! candidate BRIDGE packet, verify the packet locally, and keep the artifact in
//! `Candidate` state until separate semantic checks or human approval promote
//! it. The intent library is a named index over those content ids, so golden
//! artifacts can be fetched instead of recompiled.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod eval;
mod frame_propose;
mod intent;
mod library;
mod lift;
mod lift_frontier;
mod normalize;
mod packet_artifact;
mod resolve;
mod route;
mod shape_infer;
mod verify;

#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod frame_propose_tests;
#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod verify_tests;

pub use eval::{
    ArmMetrics, EvalArm, EvalCase, EvalCassette, EvalPlayback, EvalReport, run_eval,
    standard_eval_arms, standard_eval_corpus,
};
pub use frame_propose::{
    FrameSpecProposal, approve_frame_proposal, propose_frame, proposed_frame_part,
};
pub use intent::{CompiledIntent, IntentStatus};
pub use library::IntentLibrary;
pub use lift::{LiftOptions, forge_lift_once};
pub use lift_frontier::forge_lift_frontier;
pub use normalize::normalize_prose;
pub use packet_artifact::store_packet_artifact;
pub use resolve::{ForgeResolver, PromotePolicy, forge_resolve, forge_resolve_with_options};
pub use route::{
    RouteAttempt, RouteAttemptStatus, RoutePolicy, RouteProvenance, RouteTarget, RoutedAnswer,
    run_intent_routed, run_intent_routed_report,
};
pub use shape_infer::assert_return_shape_parses;
pub use verify::{
    ProbeOracle, Verifier, VerifyCatalog, VerifyFailure, VerifyProbe, VerifyReport, verify_answer,
};

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
