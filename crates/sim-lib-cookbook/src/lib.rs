//! Runtime `cookbook:` operations for SIM.
//!
//! This lib exposes the [`sim_cookbook`] engine as registered runtime
//! operations over a shared recipe store: `cookbook:books|chapters|list|show|
//! setup|search|next|reload` (ungated) and `cookbook:run` (read-eval gated,
//! decodes a recipe's setup through its codec, evaluates it, and checks
//! declared expectations).
//!
//! The engine stays in `sim-cookbook` (kernel-free); this crate holds the
//! kernel integration so the boundary stays clean. CLI, WebUI, browse/help,
//! and agent cookbook surfaces should call these operations or the same seeded
//! store helpers instead of creating a second projection path.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod build;
mod cli;
mod ops;
mod run;
mod runtime;
#[cfg(feature = "seed-recipes")]
mod seeds;

pub use ops::{CookbookOp, OpKind};
pub use run::{decode_setup, missing_requires, require_eval_capability, run_recipe};
pub use runtime::{
    CookbookLib, CookbookStoreHandle, install_cookbook_lib, manifest_name, op_exports, store_symbol,
};
#[cfg(feature = "seed-recipes")]
pub use seeds::{SEEDED_RECIPE_BOOKS, install_seeded_cookbook_lib, seeded_recipe_store};

#[cfg(test)]
mod tests;
