//! Model Context Protocol (MCP) surface for skills.
//!
//! Projects skills onto MCP concepts -- tools, resources, and prompts -- and
//! back. Provides the descriptor and parameter types for each MCP method, the
//! `skill/mcp-tools` and `skill/mcp-call` operations, and a deterministic
//! [`FixtureMcpTransport`] for tests. Gated behind the `mcp` feature.

/// MCP JSON-RPC message types, re-exported from `sim-codec-mcp`.
pub mod messages;
/// MCP prompt descriptors and parameters.
pub mod prompts;
/// MCP resource descriptors and parameters.
pub mod resources;
/// MCP tool descriptors, call parameters, and results.
pub mod tools;

mod fixture;
pub(crate) mod ops;

pub use fixture::FixtureMcpTransport;
pub use ops::{skill_mcp_call_symbol, skill_mcp_tools_symbol};
pub use prompts::{
    McpPromptArgument, McpPromptDescriptor, McpPromptGetParams, mcp_prompt_argument_class_symbol,
    mcp_prompt_descriptor_class_symbol, mcp_prompt_get_params_class_symbol,
};
pub use resources::{
    McpResourceDescriptor, McpResourceReadParams, mcp_resource_descriptor_class_symbol,
    mcp_resource_read_params_class_symbol,
};
pub use tools::{
    McpCallParams, McpToolDescriptor, McpToolResult, mcp_call_params_class_symbol,
    mcp_tool_descriptor_class_symbol, mcp_tool_result_class_symbol,
};

#[cfg(test)]
mod tests;
