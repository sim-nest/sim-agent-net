//! Compiled intent records and one-shot BRIDGE lifts for reusable packet programs.
//!
//! `sim-lib-forge` carries the stable data records that name a prose intent,
//! the normalized source content, the compiled BRIDGE packet content, and the
//! verification state that controls reuse. Its one-shot lift path asks a model
//! for a candidate BRIDGE packet, verifies the packet locally, and keeps the
//! artifact in `Candidate` state until separate semantic checks or human
//! approval promote it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod intent;
mod lift;
mod shape_infer;

#[cfg(test)]
mod tests;

pub use intent::{CompiledIntent, IntentStatus};
pub use lift::{LiftOptions, forge_lift_once};
pub use shape_infer::assert_return_shape_parses;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
