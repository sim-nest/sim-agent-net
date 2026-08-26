//! Compatibility adapter for initialize-era MCP connections.
//!
//! This crate deliberately depends on the stateless [`sim_lib_mcp`] service;
//! the modern crate never depends on this adapter.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use sim_codec_mcp::{McpEnvelope, McpNotification, McpRequest, McpResponse};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_mcp::{CachePolicy, McpService, NegotiatedExtensions, Principal, RequestContext};

/// Cookbook recipes for the compatibility adapter.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("connection.rs");

#[cfg(test)]
mod tests;
