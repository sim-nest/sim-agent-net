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

mod intent;
mod library;
mod lift;
mod lift_frontier;
mod normalize;
mod resolve;
mod shape_infer;

#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod tests;

pub use intent::{CompiledIntent, IntentStatus};
pub use library::IntentLibrary;
pub use lift::{LiftOptions, forge_lift_once};
pub use lift_frontier::forge_lift_frontier;
pub use normalize::normalize_prose;
pub use resolve::{ForgeResolver, PromotePolicy, forge_resolve, forge_resolve_with_options};
pub use shape_infer::assert_return_shape_parses;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
