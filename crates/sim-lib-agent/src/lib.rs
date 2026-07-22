//! Agent runtime surfaces for SIM: agents, tools, memory, patterns, fixtures,
//! fairness facets, and model-fabric contract re-exports.
//!
//! Model placement is resolved through the `model/at` catalog. In-process
//! runners and registry `site` exports share that catalog: a loaded native or
//! wasm library can register an opaque site value under a placement symbol, and
//! this crate adapts that value into an `EvalSite` without adding site behavior
//! to the kernel. `model/cached` wraps placements in a content-addressed
//! cassette, so successful inference replies are replayable by normalized
//! request content, model identity, parameters, result shape, and capabilities.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod agents;
pub mod atelier;
mod capabilities;
mod cli;
mod components;
mod config_probe;
#[cfg(feature = "cookbook")]
mod cookbook_tools;
mod core_tools;
mod embed;
mod fairness;
mod functions;
mod memory;
mod model_privacy;
mod multimodal;
mod pattern;
mod planning;
mod reply;
mod roles;
mod runner_projection;
/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
mod tool_projection;
mod tools;
mod util;

use sim_kernel::{
    AbiVersion, CapabilityName, Cx, Expr, Lib, LibManifest, LibTarget, Linker, Result, Symbol,
    Version,
};
use sim_lib_server::{
    EvalSite, install_server_lib, register_address_resolver, register_line_driver,
};

const AGENT_LIB_ID: &str = "agent";
const SANDBOX_CAPABILITY: &str = "sandbox";
const SANDBOX_SUBPROCESS_CAPABILITY: &str = "sandbox-subprocess";
const SANDBOX_WASM_CAPABILITY: &str = "sandbox-wasm";
const VOICE_CAPABILITY: &str = "voice";
const AGENT_SPAWN_CAPABILITY: &str = "agent-spawn";
const AGENT_REPLACE_CAPABILITY: &str = "agent-replace";
const AGENT_REFLECT_CAPABILITY: &str = "agent-reflect";
const SWARM_LAUNCH_CAPABILITY: &str = "swarm-launch";
const TOOL_EXPORT_KIND: &str = "tool";
/// Capability name granting an agent the right to drive an AI model runner.
pub const AI_RUNNER_CAPABILITY: &str = "ai-runner";
/// Capability name granting an AI runner outbound network access.
pub const AI_RUNNER_NETWORK_CAPABILITY: &str = "ai-runner-network";
/// Capability name granting an AI runner access to a local (on-host) model.
pub const AI_RUNNER_LOCAL_CAPABILITY: &str = "ai-runner-local";
/// Capability name granting an AI runner access to secret material (e.g. API keys).
pub const AI_RUNNER_SECRET_CAPABILITY: &str = "ai-runner-secret";
/// Capability name granting an AI runner use of a response cache.
pub const AI_RUNNER_CACHE_CAPABILITY: &str = "ai-runner-cache";
/// Capability name granting an AI runner permission to emit raw request/response logs.
pub const AI_RUNNER_RAW_LOG_CAPABILITY: &str = "ai-runner-raw-log";
/// Capability name granting authority to register or replace model placements.
pub const AI_RUNNER_PLACEMENT_CAPABILITY: &str = "ai-runner-placement";

#[cfg(test)]
pub(crate) use agents::component_name;
pub use agents::{Agent, AgentFabric, AgentManifest, AgentRef, Budget, ComponentRef, TopologyRef};
use agents::{agent_line_driver_factory, resolve_agent_address};
pub use atelier::{
    AgentMission, AtelierAction, AtelierBackend, CONTRACT_NATIVE_SCHEMA,
    ContractNativeAtelierReport, ContractNativeDeckSummary, ContractNativeGrammarSummary,
    ContractNativeGuardDenial, ContractNativeProjectionSummary, ContractNativeRouteAttempt,
    GuardCapability, GuardDecision, GuardEvaluation, GuardRefusal, RadarChunk, RadarError,
    RadarHint, RadarIndex, RadarQuery, RadarReport, RadarResult, SelfHostingScenario, SourceSpan,
    cassette_content_hash, contract_native_guard_denials, deterministic_contract_native_report,
    evaluate_guarded_action, guard_action, retrieve_radar_hints, self_hosting_scenarios,
    validate_self_hosting_scenarios,
};
pub(crate) use capabilities::{
    edit_capability, exec_capability, find_capability, fs_read_capability, fs_write_capability,
    net_http_capability, require_component_capability, require_fs_read_capability,
    require_fs_write_capability, require_net_http_capability,
};
use cli::{agent_cli_exports, register_agent_cli};
pub use components::RunnerBackend;
pub(crate) use components::{
    AgentComponent, ComponentBackend, RecorderBackend, capabilities_option, component_kind_symbol,
    maybe_f64_option, maybe_u32_option, parse_component_options, path_option, string_option,
    symbol_option,
};
pub use config_probe::{
    AgentModelConfigProbe, AgentModelProviderPresence, agent_model_config_probe_symbol,
    model_defaults_config_lib_symbol,
};
pub(crate) use embed::{cosine, embed};
pub use fairness::*;
use functions::{agent_exports, register_agent_functions};
pub(crate) use memory::lock_entries;
pub(crate) use memory::resolve_memory_backend;
pub use memory::{
    BlackboardMemory, FileMemory, MemoryBackend, MemoryKind, PersonaMemory, VectorMemory,
    WorkingMemory,
};
pub use multimodal::{
    FakeAsrFixture, FakeAsrRunner, FakeAsrSegment, FakeSensorFrame, FakeSensorStream,
    FakeVisionFixture, FakeVisionRunner,
};
pub use pattern::{AgentPattern, AgentPatternSlot};
pub use planning::{
    Decomposition, PlanningOutput, PlanningTask, Reflection, decompose, decompose_and_run, reflect,
};
pub use roles::{AgentRole, stamp_envelope_role, stamp_frame_role};
pub use runner_projection::{ExternalRunnerSpec, external_runner_value};
pub use sim_lib_agent_runner_core::{
    ModelBid, ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
    ModelUsage,
};
pub use tool_projection::{ToolSpec, install_tool};
pub use tools::Tool;
pub(crate) use tools::ToolFilter;
pub(crate) use util::{
    installed_codecs, keyword, string_from_value, stringish_from_value, symbol_from_value,
    u32_from_expr, u32_from_value, value_from_expr,
};

/// The agent runtime library, registering agent functions, tools, and CLI
/// exports as a loadable SIM [`Lib`].
pub struct AgentLib;

impl Lib for AgentLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::new(AGENT_LIB_ID),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: {
                let mut exports = agent_exports();
                exports.extend(agent_cli_exports());
                exports
            },
        }
    }

    fn load(&self, cx: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        register_agent_functions(cx, linker)?;
        register_agent_cli(cx, linker)
    }
}

/// Installs the agent library into `cx`, pulling in the server lib, registering
/// the agent address resolver and line driver, and loading [`AgentLib`].
pub fn install_agent_lib(cx: &mut Cx) -> Result<()> {
    install_server_lib(cx)?;
    register_address_resolver(Symbol::new("agent"), resolve_agent_address)?;
    register_line_driver(Symbol::new("agent"), agent_line_driver_factory)?;
    sim_lib_core::install_once(cx, &AgentLib)?;
    core_tools::install_core_tools(cx)?;
    #[cfg(feature = "cookbook")]
    cookbook_tools::install_cookbook_tools(cx)?;
    Ok(())
}

/// A pluggable agent building block (runner, tool, memory, planner, and so on)
/// that participates in evaluation as an [`EvalSite`].
pub trait Component: EvalSite {
    /// Returns the kind of component this is.
    fn kind(&self) -> ComponentKind;
    /// Returns the component's name symbol.
    fn name(&self) -> &Symbol;
    /// Returns the capabilities this component requires or grants.
    fn capabilities(&self) -> &[CapabilityName];
    /// Produces a self-describing expression for inspection of this component.
    fn reflect(&self, cx: &mut Cx) -> Result<Expr>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The role a [`Component`] plays within an agent.
pub enum ComponentKind {
    /// Drives a model to produce responses.
    Runner,
    /// Exposes a callable tool the agent may invoke.
    Tool,
    /// Stores and retrieves agent memory.
    Memory,
    /// Decomposes tasks and plans agent execution.
    Planner,
    /// Scores or ranks candidate responses.
    Judge,
    /// Routes work among targets.
    Router,
    /// Shapes the agent's voice or style.
    Persona,
    /// Retrieves external context or documents.
    Retriever,
    /// Confines execution within a sandbox.
    Sandbox,
    /// Records transcripts, audits, or metrics.
    Recorder,
    /// Handles speech synthesis or recognition.
    Voice,
    /// Describes the agent network's topology.
    Topology,
    /// A custom component kind named by the given symbol.
    Custom(Symbol),
}
