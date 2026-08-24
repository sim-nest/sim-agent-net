//! Policy-gated, content-addressed web capture.
#![forbid(unsafe_code)]

mod engine;
mod projection;
mod store;

pub use engine::*;

/// Network-free cookbook descriptors embedded for discovery.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
