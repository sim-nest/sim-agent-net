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
mod ceiling;
mod events;
mod expr;
mod fence;
mod grammar;
mod model;
mod output;
mod terminal;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

pub use card::{ModelBid, ModelCard, model_bid_class_symbol, model_card_class_symbol};
pub use ceiling::effective_ceiling;
pub use events::{ModelEvent, ModelEventSink, VecEventSink, model_request_kernel_event};
pub use fence::{FENCE_DATA_RULE, InjectionFence, fenced_data_text};
pub use grammar::shape_to_grammar;
pub use model::{
    ModelRequest, ModelResponse, ModelRunner, ModelUsage, model_request_class_symbol,
    model_response_class_symbol, model_usage_class_symbol,
};
pub use output::{
    OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
    OutputContract, RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA, grammar_dialect_from_symbol,
    grammar_dialect_symbol,
};
pub use terminal::terminal_model_content;
