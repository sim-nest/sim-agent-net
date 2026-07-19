//! Compiled intent records, BRIDGE lifts, and reusable packet-program lookup.
//!
//! `sim-lib-forge` is the FORGE intent compiler: it turns a plain-English task
//! into a reusable, verified, cached BRIDGE packet-program. It sits on top of the
//! BRIDGE envelope (`sim-codec-bridge` + `sim-lib-bridge`) and adds the
//! compile / verify / cache / route layer around it. Nothing here is
//! agent-specific -- a compiled intent is an ordinary SIM value any caller can
//! run.
//!
//! # Pipeline
//!
//! ```text
//! prose
//!   -> lift        `forge_lift_once` | `forge_lift_frontier`  (prose -> candidate BridgePacket)
//!   -> type check  `assert_return_shape_parses`               (a lift MUST type its own output)
//!   -> rx check    (BRIDGE structural validation, from sim-lib-bridge)
//!   -> verify      `verify_answer` over a `VerifyCatalog`     (assertion | judge | evidence)
//!   -> promote     `PromotePolicy`                            (Candidate -> Verified -> Golden)
//!   -> store       `IntentLibrary`                            (named index over the content store)
//!   -> resolve     `forge_resolve`                            (a Golden hit skips the model)
//!   -> route       `run_intent_routed`                        (cheap-first, escalate on failure)
//! ```
//!
//! # Key API
//!
//! - Artifact: `CompiledIntent` / `IntentStatus` (name, version, source-prose cid,
//!   packet cid, verifier ids, approval state) and `IntentLibrary`, the named
//!   index that makes reuse a lookup instead of a recompile.
//! - Lift: `forge_lift_once` (one-shot) and `forge_lift_frontier` (built
//!   part-by-part through the shared BRIDGE `frontier`, each step
//!   flat-grammar-constrained and checked), with `normalize_prose` and
//!   `assert_return_shape_parses`.
//! - Reuse: `forge_resolve` / `ForgeResolver` / `PromotePolicy` -- a matching
//!   Golden intent is fetched instead of re-lifted.
//! - Verify: `verify_answer` / `Verifier` / `VerifyCatalog` -- assertion, judge
//!   (a BRIDGE COLLAB vote), and evidence checks, so the checker catches a
//!   well-formed but *wrong* answer, not only a malformed one.
//! - Route: `run_intent_routed` / `RoutePolicy` -- run a cheap model first and
//!   escalate on verifier failure (the safe model downshift).
//! - Measure: `run_eval` / `EvalCase` / `standard_eval_corpus` -- accuracy, token
//!   cost, and model-call count across raw / compiled / cached / downshifted arms.
//! - Grow: `propose_frame` extends the BRIEF frame vocabulary from unmapped
//!   intent; `forge_verb` / `ForgeLib` is the `sim forge` Bootloader verb
//!   (lift -> review the inferred Shape -> promote -> run).
//!
//! # Why compile prose at all
//!
//! The compiler is itself a model, so a lift is a *candidate*, never trusted on
//! sight: it is validated by the BRIDGE checker and only promoted to a reusable
//! Golden once it passes. Precision comes from freezing and typing the contract
//! (a checked return Shape, owned instruction text, no injection channel); speed
//! comes from caching the Golden (a hit skips inference entirely) and from
//! downshifting to a cheaper model behind a checker that actually catches wrong.
//! The model never gets smarter -- the envelope gets reliable and the artifact
//! gets reused.
//!
//! Named out of scope: semantic prose lookup. `normalize_prose` keys reuse on a
//! byte-level normal form, not on meaning, so two differently-worded but
//! equivalent prompts do not share a Golden.

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
mod verb;
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
mod verb_tests;
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
pub use verb::{ForgeLib, forge_entrypoint_symbol, forge_verb};
pub use verify::{
    ProbeOracle, Verifier, VerifyCatalog, VerifyFailure, VerifyProbe, VerifyReport, verify_answer,
};

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
