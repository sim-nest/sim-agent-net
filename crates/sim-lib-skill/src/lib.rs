//! Agent skills for the SIM runtime.
//!
//! A *skill* is a callable capability that an agent can invoke -- a tool, a
//! model, a resource, a prompt, a memory, a retriever, a judge, or a router.
//! Each skill is described by a [`SkillCard`] that pairs a stable identity and
//! symbol with input and output [`sim_shape`] contracts, a [`SkillPolicy`]
//! governing caching/recording/privacy, and the transport that actually runs
//! the call. Cards are bound into a [`SkillRegistry`] and exposed as ordinary
//! callable runtime objects through [`SkillCallable`].
//!
//! Behavior is provided by a [`SkillTransport`]: a backend (fixture, HTTP, MCP,
//! process, OpenAI-compatible server, ...) that knows how to discover and call
//! skills. [`SkillLib`] registers the `skill/*` operations (install, bind,
//! list, card, call, and the feature-gated audit/tool/runner/serve surfaces)
//! when the library is loaded into a [`sim_kernel::Cx`].
//!
//! Optional Cargo features select transports and integrations: `mcp`, `http`,
//! `openai`, `process`, `agent`, `runner`, `serve`, and the `cache`/`cassette`
//! recording surfaces.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "agent")]
mod agent;
mod browse;
mod callable;
mod card;
mod citizen;
mod expr;
mod fixture;
#[cfg(feature = "http")]
pub mod http;
mod install;
mod manifest;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "openai")]
mod openai;
mod ops;
#[cfg(feature = "process")]
pub mod process;
#[cfg(any(feature = "cache", feature = "cassette"))]
mod record;
mod registry;
#[cfg(feature = "runner")]
mod runner;
#[cfg(feature = "serve")]
mod serve;
mod transport;

#[cfg(test)]
mod tests;

#[cfg(feature = "agent")]
pub use agent::skill_as_tool_symbol;
pub use callable::SkillCallable;
pub use card::{
    FixtureSkillSpec, SkillCacheMode, SkillCard, SkillCassetteMode, SkillPolicy,
    SkillPrivacyPolicy, SkillRole,
};
pub use citizen::{SkillCardDescriptor, skill_card_descriptor_class_symbol};
pub use fixture::{FixtureBehavior, FixtureTransport};
pub use install::{install_fixture_skill, install_skill_lib};
pub use manifest::{SkillLib, manifest_name, skill_exports};
#[cfg(feature = "mcp")]
pub use mcp::{
    FixtureMcpTransport, McpCallParams, McpPromptArgument, McpPromptDescriptor, McpPromptGetParams,
    McpResourceDescriptor, McpResourceReadParams, McpToolDescriptor, McpToolResult,
    mcp_call_params_class_symbol, mcp_prompt_argument_class_symbol,
    mcp_prompt_descriptor_class_symbol, mcp_prompt_get_params_class_symbol,
    mcp_resource_descriptor_class_symbol, mcp_resource_read_params_class_symbol,
    mcp_tool_descriptor_class_symbol, mcp_tool_result_class_symbol, skill_mcp_call_symbol,
    skill_mcp_tools_symbol,
};
#[cfg(feature = "openai")]
pub use openai::{
    insert_skill_openai_tools, skill_openai_tool_descriptor, skill_openai_tool_symbol,
    skill_openai_tools_symbol,
};
pub use ops::{
    SkillFunction, skill_bind_symbol, skill_call_capability, skill_call_symbol, skill_card_symbol,
    skill_install_capability, skill_install_symbol, skill_list_symbol, skill_serve_capability,
    skill_specific_call_capability,
};
#[cfg(any(feature = "cache", feature = "cassette"))]
pub use ops::{skill_audit_capability, skill_audit_symbol};
#[cfg(feature = "process")]
pub use process::{
    ProcessSkillProtocol, ProcessSkillSpec, ProcessSkillTransport, skill_process_capability,
};
pub use registry::{SkillRegistry, skill_registry, skill_registry_symbol};
#[cfg(feature = "runner")]
pub use runner::{SkillModelRunner, skill_as_runner_symbol, skill_model_runner};
#[cfg(feature = "serve")]
pub use serve::skill_serve_mcp_symbol;
pub use transport::{SkillEventSink, SkillTransport, SkillTransportValue, skill_transport_value};

/// Stable identifier for the skill library and its symbol namespace.
pub const SKILL_LIB_ID: &str = "skill";
