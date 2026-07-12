//! Runtime `cookbook:` operations for SIM.
//!
//! This lib exposes the [`sim_cookbook`] engine as registered runtime
//! operations over a shared recipe store: `cookbook:books|chapters|list|show|
//! setup|search|next|reload` (ungated), `cookbook:run` (read-eval gated,
//! decodes a recipe's setup through its codec, evaluates it, and checks
//! declared expectations), and `cookbook:load-lib|unload-lib` lifecycle
//! wrappers over a host-owned loadable-lib directory.
//!
//! The engine stays in `sim-cookbook` (kernel-free); this crate holds the
//! kernel integration so the boundary stays clean. CLI, WebUI, browse/help,
//! and agent cookbook surfaces should call these operations or the same seeded
//! store helpers instead of creating a second projection path.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod build;
mod catalog;
mod cli;
mod loadable;
#[cfg(test)]
mod loadable_tests;
mod ops;
mod run;
mod runtime;
#[cfg(feature = "seed-recipes")]
mod seed_catalog;
#[cfg(feature = "seed-recipes")]
mod seeds;

pub use catalog::{CookbookCapabilityProfile, EmptyCatalog, LibCatalog, load_requires};
pub use loadable::{
    LibFactory, LifecycleAction, LoadableLibEntry, LoadableLibList, lifecycle_action,
    projected_recipe_store, run_lifecycle_action, run_recipe_with_loadable_libs,
};
pub use ops::{CookbookLifecycleOp, CookbookOp, OpKind};
pub use run::{
    decode_setup, missing_requires, require_eval_capability, run_recipe, run_recipe_twice,
    run_recipe_with_catalog,
};
pub use runtime::{
    CookbookLib, CookbookStoreHandle, install_cookbook_lib,
    install_cookbook_lib_with_loadable_libs, manifest_name, op_exports, store_symbol,
};
#[cfg(feature = "seed-recipes")]
pub use seed_catalog::SeededLibCatalog;
#[cfg(feature = "seed-recipes")]
pub use seeds::{SEEDED_RECIPE_BOOKS, install_seeded_cookbook_lib, seeded_recipe_store};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
