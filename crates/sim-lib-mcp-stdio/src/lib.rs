//! Bounded newline framing and request-owned lifetimes for MCP over stdio.
//!
//! This crate is deliberately an adapter: protocol identity stays in
//! `sim-codec-mcp`, modern behavior stays in `sim-lib-mcp`, optional
//! initialize-era state stays in `sim-lib-mcp-legacy`, and process execution
//! stays behind [`ProcessProgram`](sim_lib_agent_runner_process::ProcessProgram).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod client;
mod framing;
mod server;

pub use client::{McpProcessClient, ProcessClientError, ProcessClientOptions};
pub use framing::{FrameError, JsonLineFramer};
#[cfg(feature = "legacy")]
pub use server::LegacyDispatch;
pub use server::{
    DiagnosticSink, Dispatch, DispatchCall, DispatchError, LegacyMode, ModernDispatch,
    ServerOptions, ServerSummary, StdioServer,
};

/// Cookbook recipes embedded from this package.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
