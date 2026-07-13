use super::runtime::AgentFnKind;
use sim_kernel::{Export, Symbol};
use std::sync::LazyLock;

pub(crate) fn agent_exports() -> Vec<Export> {
    AgentFnKind::all()
        .iter()
        .map(|(symbol, _)| Export::Function {
            symbol: symbol.clone(),
            function_id: None,
        })
        .collect()
}
pub(super) static AGENT_FUNCTIONS: LazyLock<Vec<(Symbol, AgentFnKind)>> = LazyLock::new(|| {
    vec![
        (Symbol::qualified("agent", "defun"), AgentFnKind::Defun),
        (Symbol::qualified("agent", "tools"), AgentFnKind::Tools),
        (
            Symbol::qualified("agent", "call-tool"),
            AgentFnKind::CallTool,
        ),
        (Symbol::qualified("agent", "make"), AgentFnKind::Make),
        (Symbol::qualified("agent", "start"), AgentFnKind::Start),
        (Symbol::qualified("agent", "connect"), AgentFnKind::Connect),
        (Symbol::qualified("agent", "call"), AgentFnKind::Call),
        (Symbol::qualified("agent", "stream"), AgentFnKind::Stream),
        (
            Symbol::qualified("agent", "component"),
            AgentFnKind::Component,
        ),
        (
            Symbol::qualified("agent", "components"),
            AgentFnKind::Components,
        ),
        (Symbol::qualified("agent", "replace"), AgentFnKind::Replace),
        (Symbol::qualified("agent", "restart"), AgentFnKind::Restart),
        (Symbol::qualified("agent", "derive"), AgentFnKind::Derive),
        (Symbol::qualified("agent", "lisp"), AgentFnKind::Lisp),
        (Symbol::qualified("agent", "reflect"), AgentFnKind::Reflect),
        (Symbol::qualified("agent", "server"), AgentFnKind::Server),
        (Symbol::qualified("agent", "attach"), AgentFnKind::Attach),
        (Symbol::qualified("agent", "audit"), AgentFnKind::Audit),
        (Symbol::qualified("agent", "trace"), AgentFnKind::Trace),
        (Symbol::qualified("agent", "wire"), AgentFnKind::Wire),
        (Symbol::qualified("agent", "pattern"), AgentFnKind::Pattern),
        (
            Symbol::qualified("memory", "working"),
            AgentFnKind::MemoryWorking,
        ),
        (Symbol::qualified("memory", "file"), AgentFnKind::MemoryFile),
        (
            Symbol::qualified("memory", "vector"),
            AgentFnKind::MemoryVector,
        ),
        (
            Symbol::qualified("memory", "blackboard"),
            AgentFnKind::MemoryBlackboard,
        ),
        (
            Symbol::qualified("memory", "persona"),
            AgentFnKind::MemoryPersona,
        ),
        (
            Symbol::qualified("memory", "append"),
            AgentFnKind::MemoryAppend,
        ),
        (
            Symbol::qualified("memory", "recent"),
            AgentFnKind::MemoryRecent,
        ),
        (Symbol::qualified("memory", "scan"), AgentFnKind::MemoryScan),
        (
            Symbol::qualified("memory", "search"),
            AgentFnKind::MemorySearch,
        ),
        (
            Symbol::qualified("memory", "snapshot"),
            AgentFnKind::MemorySnapshot,
        ),
        (
            Symbol::qualified("memory", "restore"),
            AgentFnKind::MemoryRestore,
        ),
        (Symbol::qualified("runner", "echo"), AgentFnKind::RunnerEcho),
        (
            Symbol::qualified("runner", "cassette"),
            AgentFnKind::RunnerCassette,
        ),
        (Symbol::qualified("runner", "fake"), AgentFnKind::RunnerFake),
        (
            Symbol::qualified("runner", "agent"),
            AgentFnKind::RunnerAgent,
        ),
        (
            Symbol::qualified("runner", "debate"),
            AgentFnKind::RunnerDebate,
        ),
        (
            Symbol::qualified("runner", "market"),
            AgentFnKind::RunnerMarket,
        ),
        (Symbol::qualified("runner", "card"), AgentFnKind::RunnerCard),
        (
            Symbol::qualified("runner", "cards"),
            AgentFnKind::RunnerCards,
        ),
        (
            Symbol::qualified("runner", "health"),
            AgentFnKind::RunnerHealth,
        ),
        (
            Symbol::qualified("runner", "place"),
            AgentFnKind::RunnerPlace,
        ),
        (Symbol::new("model-sites"), AgentFnKind::ModelSites),
        (Symbol::qualified("model", "at"), AgentFnKind::ModelAt),
        (
            Symbol::qualified("model", "cached"),
            AgentFnKind::ModelCached,
        ),
        (Symbol::qualified("model", "sites"), AgentFnKind::ModelSites),
        (
            Symbol::qualified("model", "site-card"),
            AgentFnKind::ModelSiteCard,
        ),
        (Symbol::new("model-policy"), AgentFnKind::ModelPolicy),
        #[cfg(feature = "runner-http")]
        (
            Symbol::qualified("provider", "profiles"),
            AgentFnKind::ProviderProfiles,
        ),
        #[cfg(feature = "runner-http")]
        (
            Symbol::qualified("provider", "probe"),
            AgentFnKind::ProviderProbe,
        ),
        #[cfg(feature = "runner-http")]
        (
            Symbol::qualified("runner", "openai-compatible"),
            AgentFnKind::RunnerOpenAiCompatible,
        ),
        #[cfg(feature = "runner-http")]
        (
            Symbol::qualified("runner", "openai"),
            AgentFnKind::RunnerOpenAi,
        ),
        #[cfg(feature = "runner-http")]
        (
            Symbol::qualified("runner", "anthropic"),
            AgentFnKind::RunnerAnthropic,
        ),
        #[cfg(feature = "runner-ollama")]
        (
            Symbol::qualified("runner", "ollama"),
            AgentFnKind::RunnerOllama,
        ),
        #[cfg(feature = "runner-process")]
        (
            Symbol::qualified("runner", "process"),
            AgentFnKind::RunnerProcess,
        ),
        (
            Symbol::qualified("planner", "budget"),
            AgentFnKind::PlannerBudget,
        ),
        (
            Symbol::qualified("planner", "refine"),
            AgentFnKind::PlannerRefine,
        ),
        (
            Symbol::qualified("planner", "parallel"),
            AgentFnKind::PlannerParallel,
        ),
        (
            Symbol::qualified("planner", "chain"),
            AgentFnKind::PlannerChain,
        ),
        (
            Symbol::qualified("judge", "rubric"),
            AgentFnKind::JudgeRubric,
        ),
        (
            Symbol::qualified("judge", "ranked-vote"),
            AgentFnKind::JudgeRankedVote,
        ),
        (
            Symbol::qualified("judge", "threshold"),
            AgentFnKind::JudgeThreshold,
        ),
        (
            Symbol::qualified("router", "round-robin"),
            AgentFnKind::RouterRoundRobin,
        ),
        (Symbol::qualified("router", "bid"), AgentFnKind::RouterBid),
        (
            Symbol::qualified("router", "sticky"),
            AgentFnKind::RouterSticky,
        ),
        (
            Symbol::qualified("persona", "style"),
            AgentFnKind::PersonaStyle,
        ),
        (
            Symbol::qualified("persona", "language"),
            AgentFnKind::PersonaLanguage,
        ),
        (
            Symbol::qualified("persona", "translator"),
            AgentFnKind::PersonaTranslator,
        ),
        (
            Symbol::qualified("retriever", "vector"),
            AgentFnKind::RetrieverVector,
        ),
        (
            Symbol::qualified("retriever", "web"),
            AgentFnKind::RetrieverWeb,
        ),
        (
            Symbol::qualified("retriever", "file"),
            AgentFnKind::RetrieverFile,
        ),
        (
            Symbol::qualified("retriever", "db"),
            AgentFnKind::RetrieverDb,
        ),
        (
            Symbol::qualified("sandbox", "wasm"),
            AgentFnKind::SandboxWasm,
        ),
        (
            Symbol::qualified("sandbox", "subprocess"),
            AgentFnKind::SandboxSubprocess,
        ),
        (
            Symbol::qualified("sandbox", "capability-restricted"),
            AgentFnKind::SandboxCapabilityRestricted,
        ),
        (
            Symbol::qualified("recorder", "journal"),
            AgentFnKind::RecorderJournal,
        ),
        (
            Symbol::qualified("recorder", "audit"),
            AgentFnKind::RecorderAudit,
        ),
        (
            Symbol::qualified("recorder", "prometheus"),
            AgentFnKind::RecorderPrometheus,
        ),
        (Symbol::qualified("voice", "tts"), AgentFnKind::VoiceTts),
        (Symbol::qualified("voice", "stt"), AgentFnKind::VoiceStt),
        (Symbol::qualified("swarm", "make"), AgentFnKind::SwarmMake),
        (
            Symbol::qualified("swarm", "launch"),
            AgentFnKind::SwarmLaunch,
        ),
        (
            Symbol::qualified("swarm", "explain"),
            AgentFnKind::SwarmExplain,
        ),
        (
            Symbol::qualified("swarm", "status"),
            AgentFnKind::SwarmStatus,
        ),
        (
            Symbol::qualified("swarm", "as-fabric"),
            AgentFnKind::SwarmAsFabric,
        ),
        (
            Symbol::qualified("swarm", "as-site"),
            AgentFnKind::SwarmAsSite,
        ),
        (
            Symbol::qualified("topology", "ring"),
            AgentFnKind::TopologyRing,
        ),
        (
            Symbol::qualified("topology", "star"),
            AgentFnKind::TopologyStar,
        ),
        (
            Symbol::qualified("topology", "mesh"),
            AgentFnKind::TopologyMesh,
        ),
        (
            Symbol::qualified("topology", "market"),
            AgentFnKind::TopologyMarket,
        ),
        (
            Symbol::qualified("topology", "debate"),
            AgentFnKind::TopologyDebate,
        ),
        (
            Symbol::qualified("topology", "speculate-verify"),
            AgentFnKind::TopologySpeculateVerify,
        ),
        (
            Symbol::qualified("topology", "open-claw"),
            AgentFnKind::TopologyOpenClaw,
        ),
        (
            Symbol::qualified("gateway", "create"),
            AgentFnKind::GatewayCreate,
        ),
    ]
});
