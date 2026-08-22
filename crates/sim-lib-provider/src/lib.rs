//! Reusable provider identity, discovery, and seat-opening contracts.
//!
//! Provider control is setup shared by every model-using application. This
//! crate deliberately stops at records and the [`ProviderAdapter`] setup seam:
//! opened seats execute through the existing [`ModelRunner`] contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod adapter;
mod auth;
mod broker;
mod call;
mod cards;
mod claude;
mod codex;
mod fanout;
mod opencode;
mod ops;
mod principal;
mod registry;
mod seat_id;
mod secret;
mod secret_provider;

pub use adapter::ProviderAdapter;
pub use auth::{
    AuthMetadata, AuthMethod, AuthOwner, SessionStatus, TermsAcknowledgement, auth_metadata_key,
};
pub use broker::{BrokerRevision, ProviderControlResult, operation as provider_operation};
pub use call::{ProviderCall, ProviderDispatch, ProviderOutcome, ProviderSeatExecution};
pub use cards::{
    EndpointCard, HarnessCard, PrincipalCard, ProviderFamilyCard, ProviderSeatCard,
    ProviderSeatLimits,
};
pub use claude::{ClaudeCliConfigHome, ClaudeCliProbe, ClaudeCliTermsPolicy, claude_cli_family};
pub use codex::{CodexCliConfigHome, CodexCliProbe, codex_cli_family};
pub use fanout::{
    Fanout, FanoutClock, FanoutMode, FanoutReport, FanoutRow, FanoutSeat, FanoutStatus,
    ManualFanoutClock, PlannedSeat, SystemFanoutClock,
};
pub use opencode::{
    OpenCodeConfig, OpenCodeProbe, OpenCodeTermsPolicy, OpenCodeTransport, opencode_cli_family,
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
