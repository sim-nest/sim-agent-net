//! Compiled intent records for reusable BRIDGE packet programs.
//!
//! `sim-lib-forge` carries the stable data records that name a prose intent,
//! the normalized source content, the compiled BRIDGE packet content, and the
//! verification state that controls reuse. The crate stores no model provider
//! behavior; later FORGE layers can load these records, check their provenance,
//! and route execution through the existing BRIDGE runtime.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod intent;

#[cfg(test)]
mod tests;

pub use intent::{CompiledIntent, IntentStatus};

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
