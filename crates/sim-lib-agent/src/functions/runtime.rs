use crate::ToolFilter;
use crate::agents::{
    agent_attach_value, agent_audit_value, agent_call_value, agent_component_value,
    agent_components_value, agent_connect_value, agent_derive_value, agent_lisp_value,
    agent_make_value, agent_reflect_value, agent_replace_value, agent_restart_value,
    agent_server_value, agent_start_value, agent_stream_value, agent_trace_value, agent_wire_value,
    gateway_create_value, swarm_as_fabric_value, swarm_as_site_value, swarm_explain_value,
    swarm_launch_value, swarm_make_value, swarm_status_value, topology_debate_value,
    topology_market_value, topology_mesh_value, topology_open_claw_value, topology_ring_value,
    topology_speculate_verify_value, topology_star_value,
};
#[cfg(feature = "runner-ollama")]
use crate::components::runner_ollama_value;
#[cfg(feature = "runner-process")]
use crate::components::runner_process_value;
use crate::components::{
    judge_ranked_vote_value, judge_rubric_value, judge_threshold_value, persona_language_value,
    persona_style_value, persona_translator_value, planner_budget_value, planner_chain_value,
    planner_parallel_value, planner_refine_value, recorder_audit_value, recorder_journal_value,
    recorder_prometheus_value, retriever_db_value, retriever_file_value, retriever_vector_value,
    retriever_web_value, router_bid_value, router_round_robin_value, router_sticky_value,
    runner_agent_value, runner_card_value, runner_cards_value, runner_cassette_value,
    runner_debate_value, runner_echo_value, runner_fake_value, runner_health_value,
    runner_market_value, runner_place_value, sandbox_capability_restricted_value,
    sandbox_subprocess_value, sandbox_wasm_value, voice_stt_value, voice_tts_value,
};
#[cfg(feature = "runner-http")]
use crate::components::{
    provider_probe_value, provider_profiles_value, runner_anthropic_value,
    runner_openai_compatible_value, runner_openai_value,
};
use crate::memory::{
    memory_append_value, memory_blackboard_value, memory_file_value, memory_persona_value,
    memory_recent_value, memory_restore_value, memory_scan_value, memory_search_value,
    memory_snapshot_value, memory_vector_value, memory_working_value,
};
use crate::pattern::agent_pattern_value;
use crate::tools::{call_tool_value, define_tool, list_tools_value, parse_tools_exprs};
use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Object, RawArgs, Result, Symbol,
    Value,
};
use std::any::Any;
#[derive(Clone, Copy)]
pub(crate) enum AgentFnKind {
    Defun,
    Tools,
    CallTool,
    Make,
    Start,
    Connect,
    Call,
    Stream,
    Component,
    Components,
    Replace,
    Restart,
    Derive,
    Lisp,
    Reflect,
    Server,
    Attach,
    Audit,
    Trace,
    Wire,
    Pattern,
    MemoryWorking,
    MemoryFile,
    MemoryVector,
    MemoryBlackboard,
    MemoryPersona,
    MemoryAppend,
    MemoryRecent,
    MemoryScan,
    MemorySearch,
    MemorySnapshot,
    MemoryRestore,
    RunnerEcho,
    RunnerCassette,
    RunnerFake,
    RunnerAgent,
    RunnerDebate,
    RunnerMarket,
    RunnerCard,
    RunnerCards,
    RunnerHealth,
    RunnerPlace,
    ModelAt,
    ModelCached,
    ModelSites,
    ModelSiteCard,
    ModelPolicy,
    #[cfg(feature = "runner-http")]
    ProviderProfiles,
    #[cfg(feature = "runner-http")]
    ProviderProbe,
    #[cfg(feature = "runner-http")]
    RunnerOpenAiCompatible,
    #[cfg(feature = "runner-http")]
    RunnerOpenAi,
    #[cfg(feature = "runner-http")]
    RunnerAnthropic,
    #[cfg(feature = "runner-ollama")]
    RunnerOllama,
    #[cfg(feature = "runner-process")]
    RunnerProcess,
    PlannerBudget,
    PlannerRefine,
    PlannerParallel,
    PlannerChain,
    JudgeRubric,
    JudgeRankedVote,
    JudgeThreshold,
    RouterRoundRobin,
    RouterBid,
    RouterSticky,
    PersonaStyle,
    PersonaLanguage,
    PersonaTranslator,
    RetrieverVector,
    RetrieverWeb,
    RetrieverFile,
    RetrieverDb,
    SandboxWasm,
    SandboxSubprocess,
    SandboxCapabilityRestricted,
    RecorderJournal,
    RecorderAudit,
    RecorderPrometheus,
    VoiceTts,
    VoiceStt,
    SwarmMake,
    SwarmLaunch,
    SwarmExplain,
    SwarmStatus,
    SwarmAsFabric,
    SwarmAsSite,
    TopologyRing,
    TopologyStar,
    TopologyMesh,
    TopologyMarket,
    TopologyDebate,
    TopologySpeculateVerify,
    TopologyOpenClaw,
    GatewayCreate,
}

impl AgentFnKind {
    pub(crate) fn all() -> &'static [(Symbol, AgentFnKind)] {
        &super::catalog::AGENT_FUNCTIONS
    }
}

#[derive(Clone)]
pub(crate) struct AgentFunction {
    pub(crate) symbol: Symbol,
    pub(crate) kind: AgentFnKind,
}

impl Object for AgentFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for AgentFunction {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for AgentFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        dispatch_value_call(self.kind, cx, args)
    }

    fn call_exprs(&self, cx: &mut Cx, args: RawArgs) -> Result<Value> {
        match self.kind {
            AgentFnKind::Defun => define_tool(cx, args.into_exprs()),
            AgentFnKind::Tools => parse_tools_exprs(cx, args.into_exprs()),
            AgentFnKind::CallTool => {
                let values = args
                    .into_exprs()
                    .into_iter()
                    .map(|expr| cx.eval_expr(expr))
                    .collect::<Result<Vec<_>>>()?;
                call_tool_value(cx, Args::new(values))
            }
            _ => {
                let values = args
                    .into_exprs()
                    .into_iter()
                    .map(|expr| cx.eval_expr(expr))
                    .collect::<Result<Vec<_>>>()?;
                dispatch_value_call(self.kind, cx, Args::new(values))
            }
        }
    }
}

fn dispatch_value_call(kind: AgentFnKind, cx: &mut Cx, args: Args) -> Result<Value> {
    match kind {
        AgentFnKind::Defun => Err(Error::Eval(
            "agent/defun must be called with unevaluated option expressions".to_owned(),
        )),
        AgentFnKind::Tools => list_tools_value(cx, ToolFilter::default()),
        AgentFnKind::CallTool => call_tool_value(cx, args),
        AgentFnKind::Make => agent_make_value(cx, args),
        AgentFnKind::Start => agent_start_value(cx, args),
        AgentFnKind::Connect => agent_connect_value(cx, args),
        AgentFnKind::Call => agent_call_value(cx, args),
        AgentFnKind::Stream => agent_stream_value(cx, args),
        AgentFnKind::Component => agent_component_value(cx, args),
        AgentFnKind::Components => agent_components_value(cx, args),
        AgentFnKind::Replace => agent_replace_value(cx, args),
        AgentFnKind::Restart => agent_restart_value(cx, args),
        AgentFnKind::Derive => agent_derive_value(cx, args),
        AgentFnKind::Lisp => agent_lisp_value(cx, args),
        AgentFnKind::Reflect => agent_reflect_value(cx, args),
        AgentFnKind::Server => agent_server_value(cx, args),
        AgentFnKind::Attach => agent_attach_value(cx, args),
        AgentFnKind::Audit => agent_audit_value(cx, args),
        AgentFnKind::Trace => agent_trace_value(cx, args),
        AgentFnKind::Wire => agent_wire_value(cx, args),
        AgentFnKind::Pattern => agent_pattern_value(cx, args),
        AgentFnKind::MemoryWorking => memory_working_value(cx, args),
        AgentFnKind::MemoryFile => memory_file_value(cx, args),
        AgentFnKind::MemoryVector => memory_vector_value(cx, args),
        AgentFnKind::MemoryBlackboard => memory_blackboard_value(cx, args),
        AgentFnKind::MemoryPersona => memory_persona_value(cx, args),
        AgentFnKind::MemoryAppend => memory_append_value(cx, args),
        AgentFnKind::MemoryRecent => memory_recent_value(cx, args),
        AgentFnKind::MemoryScan => memory_scan_value(cx, args),
        AgentFnKind::MemorySearch => memory_search_value(cx, args),
        AgentFnKind::MemorySnapshot => memory_snapshot_value(cx, args),
        AgentFnKind::MemoryRestore => memory_restore_value(cx, args),
        AgentFnKind::RunnerEcho => runner_echo_value(cx, args),
        AgentFnKind::RunnerCassette => runner_cassette_value(cx, args),
        AgentFnKind::RunnerFake => runner_fake_value(cx, args),
        AgentFnKind::RunnerAgent => runner_agent_value(cx, args),
        AgentFnKind::RunnerDebate => runner_debate_value(cx, args),
        AgentFnKind::RunnerMarket => runner_market_value(cx, args),
        AgentFnKind::RunnerCard => runner_card_value(cx, args),
        AgentFnKind::RunnerCards => runner_cards_value(cx, args),
        AgentFnKind::RunnerHealth => runner_health_value(cx, args),
        AgentFnKind::RunnerPlace => runner_place_value(cx, args),
        AgentFnKind::ModelAt => crate::components::model_at_value(cx, args),
        AgentFnKind::ModelCached => crate::components::model_cached_value(cx, args),
        AgentFnKind::ModelSites => crate::components::model_sites_value(cx, args),
        AgentFnKind::ModelSiteCard => crate::components::model_site_card_value(cx, args),
        AgentFnKind::ModelPolicy => crate::components::model_policy_value(cx, args),
        #[cfg(feature = "runner-http")]
        AgentFnKind::ProviderProfiles => provider_profiles_value(cx, args),
        #[cfg(feature = "runner-http")]
        AgentFnKind::ProviderProbe => provider_probe_value(cx, args),
        #[cfg(feature = "runner-http")]
        AgentFnKind::RunnerOpenAiCompatible => runner_openai_compatible_value(cx, args),
        #[cfg(feature = "runner-http")]
        AgentFnKind::RunnerOpenAi => runner_openai_value(cx, args),
        #[cfg(feature = "runner-http")]
        AgentFnKind::RunnerAnthropic => runner_anthropic_value(cx, args),
        #[cfg(feature = "runner-ollama")]
        AgentFnKind::RunnerOllama => runner_ollama_value(cx, args),
        #[cfg(feature = "runner-process")]
        AgentFnKind::RunnerProcess => runner_process_value(cx, args),
        AgentFnKind::PlannerBudget => planner_budget_value(cx, args),
        AgentFnKind::PlannerRefine => planner_refine_value(cx, args),
        AgentFnKind::PlannerParallel => planner_parallel_value(cx, args),
        AgentFnKind::PlannerChain => planner_chain_value(cx, args),
        AgentFnKind::JudgeRubric => judge_rubric_value(cx, args),
        AgentFnKind::JudgeRankedVote => judge_ranked_vote_value(cx, args),
        AgentFnKind::JudgeThreshold => judge_threshold_value(cx, args),
        AgentFnKind::RouterRoundRobin => router_round_robin_value(cx, args),
        AgentFnKind::RouterBid => router_bid_value(cx, args),
        AgentFnKind::RouterSticky => router_sticky_value(cx, args),
        AgentFnKind::PersonaStyle => persona_style_value(cx, args),
        AgentFnKind::PersonaLanguage => persona_language_value(cx, args),
        AgentFnKind::PersonaTranslator => persona_translator_value(cx, args),
        AgentFnKind::RetrieverVector => retriever_vector_value(cx, args),
        AgentFnKind::RetrieverWeb => retriever_web_value(cx, args),
        AgentFnKind::RetrieverFile => retriever_file_value(cx, args),
        AgentFnKind::RetrieverDb => retriever_db_value(cx, args),
        AgentFnKind::SandboxWasm => sandbox_wasm_value(cx, args),
        AgentFnKind::SandboxSubprocess => sandbox_subprocess_value(cx, args),
        AgentFnKind::SandboxCapabilityRestricted => sandbox_capability_restricted_value(cx, args),
        AgentFnKind::RecorderJournal => recorder_journal_value(cx, args),
        AgentFnKind::RecorderAudit => recorder_audit_value(cx, args),
        AgentFnKind::RecorderPrometheus => recorder_prometheus_value(cx, args),
        AgentFnKind::VoiceTts => voice_tts_value(cx, args),
        AgentFnKind::VoiceStt => voice_stt_value(cx, args),
        AgentFnKind::SwarmMake => swarm_make_value(cx, args),
        AgentFnKind::SwarmLaunch => swarm_launch_value(cx, args),
        AgentFnKind::SwarmExplain => swarm_explain_value(cx, args),
        AgentFnKind::SwarmStatus => swarm_status_value(cx, args),
        AgentFnKind::SwarmAsFabric => swarm_as_fabric_value(cx, args),
        AgentFnKind::SwarmAsSite => swarm_as_site_value(cx, args),
        AgentFnKind::TopologyRing => topology_ring_value(cx, args),
        AgentFnKind::TopologyStar => topology_star_value(cx, args),
        AgentFnKind::TopologyMesh => topology_mesh_value(cx, args),
        AgentFnKind::TopologyMarket => topology_market_value(cx, args),
        AgentFnKind::TopologyDebate => topology_debate_value(cx, args),
        AgentFnKind::TopologySpeculateVerify => topology_speculate_verify_value(cx, args),
        AgentFnKind::TopologyOpenClaw => topology_open_claw_value(cx, args),
        AgentFnKind::GatewayCreate => gateway_create_value(cx, args),
    }
}
