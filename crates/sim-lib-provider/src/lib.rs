//! Reusable provider identity, discovery, and seat-opening contracts.
//!
//! Provider control is setup shared by every model-using application. This
//! crate deliberately stops at records and the [`ProviderAdapter`] setup seam:
//! opened seats execute through the existing [`ModelRunner`] contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod adapter;
mod cards;
mod ops;
mod principal;
mod registry;
mod seat_id;
mod secret;
mod secret_provider;

pub use adapter::ProviderAdapter;
pub use cards::{
    EndpointCard, HarnessCard, PrincipalCard, ProviderFamilyCard, ProviderSeatCard,
    ProviderSeatLimits,
};
pub use ops::{discover, families, open, seats, show_family, show_seat};
pub use principal::CredentialSource;
pub use registry::ProviderRegistry;
pub use seat_id::ProviderSeatId;
pub use secret::Secret;
pub use secret_provider::{SecretProvider, SecretProviderRegistry};
pub use sim_lib_agent_runner_core::ModelRunner;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
