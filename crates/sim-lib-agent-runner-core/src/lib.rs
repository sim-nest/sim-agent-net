//! Provider-neutral runner contracts for SIM model fabrics.
//!
//! This crate defines the shared in-process objects used by SIM model runners:
//!
//! - [`ModelRequest`] for normalized task and transcript input
//! - [`ModelResponse`] for final content and usage output
//! - [`ModelEvent`] and [`ModelEventSink`] for streaming progress
//! - [`ModelCard`] and [`ModelBid`] for routing and selection
//! - [`ModelRunner`] for executable runner implementations
//!
//! Higher-level crates install concrete runners, policies, markets, and
//! agent-model adapters on top of these contracts.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod card;
mod events;
mod expr;
mod grammar;
mod model;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

pub use card::{ModelBid, ModelCard, model_bid_class_symbol, model_card_class_symbol};
pub use events::{ModelEvent, ModelEventSink, VecEventSink, model_request_kernel_event};
pub use grammar::shape_to_grammar;
pub use model::{
    ModelRequest, ModelResponse, ModelRunner, ModelUsage, model_request_class_symbol,
    model_response_class_symbol, model_usage_class_symbol,
};
