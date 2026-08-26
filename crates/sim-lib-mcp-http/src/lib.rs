//! Final-protocol Streamable HTTP adapter for stateless MCP.
//!
//! This crate owns endpoint and projection policy. Parsing, sockets, TLS,
//! generic body bounds, and backpressure stay in the composed server and
//! client organs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{io, sync::Mutex, time::Instant};

use sim_cancel::Cancellation;
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_mcp::{
    McpEnvelope, McpError, McpErrorEnvelope, PARSE_ERROR, envelope_to_expr, expr_to_envelope,
};
use sim_kernel::{
    CapabilityName, Cx, EncodeOptions, Error, Expr, ReadPolicy, Result, Symbol,
    capability::CapabilitySet,
};
use sim_lib_mcp::{CachePolicy, McpService, NegotiatedExtensions, Principal, RequestContext};
use sim_lib_mcp_legacy::LegacyConnection;
use sim_lib_net_http as net_http;
use sim_lib_oauth_core::{AccessTokenVerifier, ScopeSet, Secret, SecureUrl};
use sim_lib_provider::{AuthMethod, ProviderSeatCard};
use sim_lib_server::{
    BodyReader, RawHandler, RequestHead, RequestScope, ResponseHead, ResponseWriter,
};

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

const JSON: &str = "application/json";
const SSE: &str = "text/event-stream";
const PROTOCOL: &str = "2026-07-28";

include!("policy.rs");
include!("dispatch.rs");
include!("server.rs");
include!("client.rs");

#[cfg(test)]
mod tests;
