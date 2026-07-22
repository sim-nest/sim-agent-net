use super::super::first_codec;
use crate::{SWARM_LAUNCH_CAPABILITY, value_from_expr};
use sim_citizen_derive::non_citizen;
use sim_kernel::{
    CapabilityName, Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol, Value,
};
use sim_lib_server::{
    EvalSite, Server, ServerAddress, StreamHandle, eval_request_from_frame, server_frame_from_reply,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::runtime::{AgentDispatchSite, AgentEvalSite};

/// A runtime value referencing an agent component.
pub type ComponentRef = Value;
/// A runtime value referencing an agent.
pub type AgentRef = Value;
/// A runtime value referencing an agent-network topology.
pub type TopologyRef = Value;

/// Resource limits bounding an agent run.
#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// Maximum number of turns allowed, if bounded.
    pub max_turns: Option<u32>,
    /// Maximum accumulated cost allowed, if bounded.
    pub max_cost: Option<f64>,
}

/// The set of components composing an agent, grouped by role.
#[derive(Clone, Default)]
pub struct AgentManifest {
    /// Model runners available to the agent.
    pub runners: Vec<ComponentRef>,
    /// Tools the agent may invoke.
    pub tools: Vec<ComponentRef>,
    /// Memory backends attached to the agent.
    pub memories: Vec<ComponentRef>,
    /// Optional planner directing the agent's execution.
    pub planner: Option<ComponentRef>,
    /// Optional judge scoring candidate responses.
    pub judge: Option<ComponentRef>,
    /// Optional router dispatching work among targets.
    pub router: Option<ComponentRef>,
    /// Optional persona shaping the agent's voice or style.
    pub persona: Option<ComponentRef>,
    /// Retrievers supplying external context.
    pub retrievers: Vec<ComponentRef>,
    /// Optional sandbox confining the agent's execution.
    pub sandbox: Option<ComponentRef>,
    /// Recorders capturing transcripts, audits, or metrics.
    pub recorders: Vec<ComponentRef>,
    /// Optional voice component for speech synthesis or recognition.
    pub voice: Option<ComponentRef>,
    /// Optional topology describing the agent network.
    pub topology: Option<ComponentRef>,
    /// Additional named components keyed by symbol.
    pub extras: BTreeMap<Symbol, ComponentRef>,
}

pub(crate) struct AgentState {
    pub(crate) runtime_site: Arc<dyn EvalSite>,
    pub(crate) address: Option<ServerAddress>,
    pub(crate) server: Option<Arc<Server>>,
    pub(crate) default_codec: Symbol,
    pub(crate) supported_codecs: Vec<Symbol>,
}

#[non_citizen(
    reason = "live agent session handle; reconstruct via agent-runner/ModelRequest, server/Address, and topology/Package descriptors",
    kind = "handle",
    descriptor = "agent-runner/ModelRequest"
)]
/// A live agent session: a named manifest of components with its own
/// capabilities, isolation policy, and runtime state.
pub struct Agent {
    /// Process-unique identifier assigned when the agent is created.
    pub id: u64,
    /// The agent's name.
    pub name: Symbol,
    /// The agent's component manifest, mutable behind a lock.
    pub manifest: Mutex<AgentManifest>,
    /// Capabilities granted to the agent.
    pub capabilities: Vec<CapabilityName>,
    /// Isolation policy governing the agent's server interactions.
    pub policy: sim_lib_server::IsolationPolicy,
    pub(crate) state: Arc<Mutex<AgentState>>,
}

#[non_citizen(
    reason = "live agent fabric handle; reconstruct via topology/Package and agent-runner/ModelCard descriptors",
    kind = "handle",
    descriptor = "topology/Package"
)]
/// A swarm of agents that evaluates together as a single [`EvalFabric`],
/// coordinating its members through an optional topology, planner, and
/// shared blackboard.
pub struct AgentFabric {
    /// Process-unique identifier assigned when the fabric is created.
    pub id: u64,
    /// The fabric's name.
    pub name: Symbol,
    /// The agents participating in this fabric.
    pub members: Vec<AgentRef>,
    /// Optional topology describing how members are connected.
    pub topology: Option<TopologyRef>,
    /// Optional planner coordinating the members' work.
    pub planner: Option<ComponentRef>,
    /// Optional shared blackboard memory for the members.
    pub blackboard: Option<ComponentRef>,
    /// Optional budget bounding a fabric run.
    pub budget: Option<Budget>,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) last_run: Mutex<Option<Expr>>,
    pub(crate) runs: Arc<Mutex<SwarmRegistry>>,
}

#[derive(Clone)]
pub(crate) struct RuntimeValueSite {
    pub(crate) value: Value,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) kind: &'static str,
}

#[non_citizen(
    reason = "live agent loopback stream handle; reconstruct via server/Frame and stream/Packet descriptors",
    kind = "handle",
    descriptor = "stream/Packet"
)]
#[derive(Clone)]
pub(crate) struct LoopbackStream {
    pub(crate) handle: Arc<StreamHandle>,
}

impl LoopbackStream {
    pub(crate) fn new(handle: Arc<StreamHandle>) -> Self {
        Self { handle }
    }
}

static NEXT_AGENT_ID: AtomicU64 = AtomicU64::new(1);
pub(crate) static NEXT_SWARM_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct SwarmRunRecord {
    pub(crate) transcript: Expr,
}

#[derive(Clone)]
pub(crate) struct SwarmStatus {
    pub(crate) active: bool,
    pub(crate) task_id: Option<String>,
    pub(crate) turns_used: u32,
    pub(crate) turns_remaining: Option<u32>,
    pub(crate) cost_used: f64,
    pub(crate) cost_remaining: Option<f64>,
    pub(crate) budget_exhausted: bool,
    pub(crate) last_value: Expr,
}

impl Default for SwarmStatus {
    fn default() -> Self {
        Self {
            active: false,
            task_id: None,
            turns_used: 0,
            turns_remaining: None,
            cost_used: 0.0,
            cost_remaining: None,
            budget_exhausted: false,
            last_value: Expr::Nil,
        }
    }
}

#[derive(Default)]
pub(crate) struct SwarmRegistry {
    pub(crate) next_task_id: u64,
    pub(crate) last_task_id: Option<String>,
    pub(crate) last_run: Option<Expr>,
    pub(crate) history: BTreeMap<String, SwarmRunRecord>,
    pub(crate) status: SwarmStatus,
}

impl Agent {
    pub(crate) fn new(
        name: Symbol,
        manifest: AgentManifest,
        capabilities: Vec<CapabilityName>,
        policy: sim_lib_server::IsolationPolicy,
        codecs: Vec<Symbol>,
    ) -> Self {
        let default_codec = first_codec(&codecs);
        let site = Arc::new(AgentEvalSite {
            name: name.clone(),
            manifest: manifest.clone(),
            codecs: codecs.clone(),
            capabilities: capabilities.clone(),
        });
        Self {
            id: NEXT_AGENT_ID.fetch_add(1, Ordering::Relaxed),
            name,
            manifest: Mutex::new(manifest),
            capabilities,
            policy,
            state: Arc::new(Mutex::new(AgentState {
                runtime_site: site,
                address: None,
                server: None,
                default_codec,
                supported_codecs: codecs,
            })),
        }
    }

    pub(crate) fn site(&self) -> Result<Arc<dyn EvalSite>> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        Ok(Arc::new(AgentDispatchSite {
            state: self.state.clone(),
            address: ServerAddress::Local,
            codecs: state.supported_codecs.clone(),
        }))
    }

    pub(crate) fn manifest_clone(&self) -> Result<AgentManifest> {
        self.manifest
            .lock()
            .map(|manifest| manifest.clone())
            .map_err(|_| Error::PoisonedLock("agent manifest"))
    }
}

impl AgentFabric {
    pub(crate) fn realize_expr(&self, cx: &mut Cx, expr: Expr) -> Result<Expr> {
        cx.require(&CapabilityName::new(SWARM_LAUNCH_CAPABILITY))?;
        let result = super::swarm_realize_expr(cx, self, expr)?;
        *self
            .last_run
            .lock()
            .map_err(|_| Error::PoisonedLock("swarm state"))? = Some(result.clone());
        Ok(result)
    }
}

impl EvalFabric for AgentFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let expr = self.realize_expr(cx, request.expr)?;
        let value = value_from_expr(cx, &expr)?;
        Ok(EvalReply {
            value,
            diagnostics: cx.take_diagnostics(),
            trace: None,
        })
    }
}

impl EvalSite for AgentFabric {
    fn site_kind(&self) -> &'static str {
        "swarm"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut Cx,
        frame: sim_lib_server::ServerFrame,
    ) -> Result<sim_lib_server::ServerFrame> {
        let consistency = frame.envelope.consistency;
        let expr = eval_request_from_frame(cx, &frame)?.expr;
        let result = self.realize_expr(cx, expr)?;
        let value = value_from_expr(cx, &result)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
