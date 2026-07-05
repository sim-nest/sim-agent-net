//! Library-only MCP surface projection for SIM.
//!
//! This crate projects native browse Cards and optional `SkillCard` records
//! into redacted `McpSurfaceCard` rows. Routing, transport, and callable
//! execution are implemented by later MCP layers.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "cassette")]
mod cassette;
#[cfg(feature = "client")]
mod client;
mod content;
mod exec;
#[cfg(feature = "http")]
mod http;
mod install;
mod manifest;
mod methods;
mod native;
mod ops;
mod profile;
mod router;
#[cfg(feature = "sampling")]
mod sampling;
mod schema;
#[cfg(feature = "serve")]
mod serve;
#[cfg(all(test, feature = "stdio"))]
mod stdio_tests;
mod session;
#[cfg(feature = "skill")]
mod skill;
/// Line-delimited MCP transport over standard input and output.
#[cfg(feature = "stdio")]
pub mod stdio;
#[cfg(feature = "stream")]
mod stream;
mod surface;
mod uri;

#[cfg(all(test, feature = "cassette"))]
mod cassette_tests;
#[cfg(all(test, feature = "client"))]
mod client_tests;
#[cfg(all(test, feature = "skill"))]
mod coexistence_tests;
#[cfg(all(test, feature = "http"))]
mod http_tests;
#[cfg(test)]
mod prompts_tests;
#[cfg(test)]
mod resources_tests;
#[cfg(all(test, feature = "sampling"))]
mod sampling_tests;
#[cfg(all(test, feature = "stream", feature = "progress"))]
mod stream_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tools_call_tests;
#[cfg(test)]
mod tools_list_tests;

#[cfg(feature = "cassette")]
pub use cassette::{
    McpAuditEntry, McpCassette, McpCassetteEntry, mcp_audit_entry_class_symbol,
    mcp_cassette_class_symbol, mcp_cassette_entry_class_symbol,
};
#[cfg(feature = "client")]
pub use client::{
    McpClient, McpClientAllowPolicy, McpClientCassettePeer, McpClientPeer, McpClientTransport,
    RouterMcpPeer, mcp_client_capability,
};
pub use exec::{
    mcp_prompts_get_capability, mcp_resources_read_capability, mcp_tools_call_capability,
};
#[cfg(feature = "http")]
pub use http::{McpHttpAdapter, mcp_http_capability};
pub use install::install_mcp_lib;
pub use manifest::{McpLib, manifest_name};
pub use native::{
    McpExportFacet, McpNativeCard, NativeFacet, mcp_export_facet_name, mcp_export_operation_symbol,
    native_surface_rows,
};
pub use ops::{
    McpFunction, call_symbol, get_prompt_symbol, handle_symbol, health_symbol, initialize_symbol,
    mcp_exports, prompts_symbol, read_symbol, resources_symbol, tools_symbol,
};
pub use profile::McpProfile;
pub use router::{McpRouter, RouterReply};
#[cfg(feature = "sampling")]
pub use sampling::{
    FixtureSamplingHost, McpSamplingHost, McpSamplingRunner, McpSamplingRunnerValue,
    mcp_sampling_capability, mcp_sampling_data_kind, mcp_sampling_runner_symbol,
    sampling_runner_value,
};
pub use schema::shape_to_json_schema;
#[cfg(feature = "serve")]
pub use serve::{CliOptions, McpServeLib, Transport, mcp_bootloader, mcp_serve_entrypoint_symbol};
pub use session::McpSession;
#[cfg(feature = "skill")]
pub use skill::{project_skill_surface, skill_surface_rows};
#[cfg(feature = "stream")]
pub use stream::{
    mcp_cancelled_data_kind, mcp_failed_diagnostic_kind, mcp_overflowed_diagnostic_kind,
    mcp_progress_data_kind, mcp_truncated_diagnostic_kind,
};
pub use surface::{
    McpAnnotation, McpAnnotationVisibility, McpStreamPolicy, McpSurfaceCard, McpSurfaceRole,
    McpSurfaceSource, project_mcp_surface, project_native_surface, project_surface_rows,
    stable_mcp_name,
};

/// Stable identifier under which this library registers in the runtime.
pub const MCP_LIB_ID: &str = "mcp";
