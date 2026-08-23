//! Modern-first, transport-neutral MCP client.
//!
//! The client composes the existing HTTP and stdio bindings through
//! [`BindingPeer`]. It probes before the first application request, imports the
//! canonical SIM [`SkillCard`](sim_lib_skill::SkillCard), and never retries an
//! application request merely to switch protocol eras.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod client;
mod identity;
mod model;
mod schema;

pub use cache::{CacheDisposition, CacheKey, ClientCache, MemoryLruCache};
pub use client::{Client, ClientPolicy, IconDescriptor, McpCallable};
pub use identity::{EndpointIdentity, HttpEndpoint};
pub use model::{
    BindingError, BindingPeer, CallContext, ClientError, ClientEvent, ClientLedger, Discovery, Era,
    InputBroker, InputRequest, Invocation, Outcome, PeerReply, PersistentCache, Subscription,
};
pub use schema::SchemaContract;

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
