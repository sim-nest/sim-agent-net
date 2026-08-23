//! Reusable, single-attempt agent model and tool step boundaries.

use std::sync::Arc;

use sim_codec_bridge::BridgePacket;
use sim_kernel::{CapabilityName, Cx, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_conduct::run_frame_shape;
use sim_lib_agent_conduct_core::{
    AgentEvent, AgentRunFrame, AgentStepCard, AgentUsageBudget, UsageQuantity,
};
use sim_lib_agent_runner_core::{ModelResponse, ModelRunner};
use sim_lib_bridge::{AskAttempt, run_ask_once};
use sim_lib_provider::{ProviderRegistry, ProviderSeatId};

use crate::util::value_from_expr;

/// Immutable options captured by a model-turn node factory.
#[derive(Clone, Debug)]
pub struct ModelTurnOptions {
    /// Exact provider seat selected by the manifest binding.
    pub seat: ProviderSeatId,
    /// Provider-owned, non-secret open options.
    pub open_options: Expr,
    /// Effective already-narrowed budget.
    pub budget: AgentUsageBudget,
    /// Admission charge reserved before invoking the runner.
    pub charge: UsageQuantity,
}

/// Typed result of exactly one model exchange.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelTurnResult {
    /// A checked final BRIDGE packet and its redacted journal event.
    Final {
        packet: BridgePacket,
        event: AgentEvent,
    },
    /// A checked reply asks the graph to execute tool calls next.
    ToolCalls {
        packet: BridgePacket,
        event: AgentEvent,
    },
    /// The reply requires a later graph-owned repair step.
    RepairNeeded {
        failure: sim_lib_bridge::AskFailure,
        event: AgentEvent,
    },
    /// Admission refused before opening or invoking the provider seat.
    BudgetExhausted,
    /// A redaction-safe execution failure.
    Error(String),
}

struct RunnerFabric(Arc<dyn ModelRunner>);

impl EvalFabric for RunnerFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let response = self.0.infer_request(cx, request)?;
        Ok(EvalReply {
            value: value_from_expr(cx, &Expr::from(response))?,
            diagnostics: cx.take_diagnostics(),
            trace: None,
        })
    }
}

/// Executes one checked BRIDGE model exchange through one explicitly selected registry seat.
///
/// Admission happens before the registry is asked to open the seat. The frame records only
/// stable seat and redacted principal identities plus packet references; endpoint, credential,
/// transport, preference, and hidden reasoning never enter conduct state.
pub fn execute_model_turn_once(
    cx: &mut Cx,
    registry: &ProviderRegistry,
    frame: &mut AgentRunFrame,
    packet: BridgePacket,
    options: &ModelTurnOptions,
) -> Result<ModelTurnResult> {
    if options.budget.admit(&frame.usage, &options.charge).is_err() {
        frame.outcome = Symbol::qualified("agent.stop", "budget-exhausted");
        return Ok(ModelTurnResult::BudgetExhausted);
    }
    let Some(card) = registry.show_seat(&options.seat) else {
        return Ok(ModelTurnResult::Error(format!(
            "provider seat {} has not been discovered",
            options.seat
        )));
    };
    frame.state.upsert(
        Symbol::qualified("agent.provider", "seat"),
        Expr::String(card.seat.to_string()),
    )?;
    frame.state.upsert(
        Symbol::qualified("agent.provider", "principal"),
        Expr::String(card.principal.digest.clone()),
    )?;
    let runner = registry.open(cx, &options.seat, options.open_options.clone())?;
    frame
        .usage
        .charge(&options.budget, options.charge.clone())
        .expect("the unchanged charge was admitted immediately before opening the seat");
    let packet_ref = packet.header.cid.clone().map_or(Expr::Nil, Expr::String);
    match run_ask_once(cx, &RunnerFabric(runner), packet)? {
        AskAttempt::Answer(packet) => {
            let event = AgentEvent::new(
                Symbol::qualified("agent.event", "model-final"),
                Expr::Map(vec![(Expr::Symbol(Symbol::new("packet")), packet_ref)]),
            );
            frame.working = Expr::String(
                packet
                    .header
                    .cid
                    .clone()
                    .unwrap_or_else(|| "unstamped".to_owned()),
            );
            if packet_contains_tool_calls(&packet) {
                frame.outcome = Symbol::new("tool-calls");
                Ok(ModelTurnResult::ToolCalls { packet, event })
            } else {
                frame.outcome = Symbol::new("final");
                Ok(ModelTurnResult::Final { packet, event })
            }
        }
        AskAttempt::RepairNeeded { failure, .. } => {
            frame.outcome = Symbol::new("error");
            Ok(ModelTurnResult::RepairNeeded {
                event: AgentEvent::new(
                    Symbol::qualified("agent.event", "model-repair-needed"),
                    Expr::Map(vec![(Expr::Symbol(Symbol::new("packet")), packet_ref)]),
                ),
                failure,
            })
        }
    }
}

fn packet_contains_tool_calls(packet: &BridgePacket) -> bool {
    format!("{:?}", packet.body).contains("tool-calls")
}

/// Stateless factory configuration for the two standard bound step targets.
#[derive(Clone, Debug)]
pub struct AgentStepTargetFactory {
    /// Role resolved from the manifest's existing node binding.
    pub role: Symbol,
    /// Node options captured at binding time; never run state.
    pub node_options: Expr,
}

impl AgentStepTargetFactory {
    /// Constructs a factory after the manifest binding has resolved the node role.
    pub fn new(role: Symbol, node_options: Expr) -> Self {
        Self { role, node_options }
    }
}

/// Card for the reusable `agent.step/model-turn` target.
pub fn model_turn_card() -> AgentStepCard {
    AgentStepCard {
        step_id: Symbol::qualified("agent.step", "model-turn"),
        version: 1,
        input_shape: run_frame_shape(),
        output_shape: run_frame_shape(),
        roles: vec![Symbol::new("runner")],
        capabilities: vec![CapabilityName::new(crate::AI_RUNNER_CAPABILITY)],
        outcomes: vec![
            Symbol::new("tool-calls"),
            Symbol::new("final"),
            Symbol::new("error"),
        ],
        may_request_effect: true,
        usage_dimensions: vec![Symbol::qualified("agent.usage", "model-turn")],
        redaction: Symbol::new("packet-references"),
        replay: Symbol::new("effect-safe"),
    }
}

/// Card for the reusable `agent.step/tool-batch` target.
pub fn tool_batch_card() -> AgentStepCard {
    AgentStepCard {
        step_id: Symbol::qualified("agent.step", "tool-batch"),
        version: 1,
        input_shape: run_frame_shape(),
        output_shape: run_frame_shape(),
        roles: vec![Symbol::new("tools")],
        capabilities: vec![],
        outcomes: vec![
            Symbol::new("continue"),
            Symbol::new("final"),
            Symbol::new("error"),
        ],
        may_request_effect: true,
        usage_dimensions: vec![Symbol::qualified("agent.usage", "tool-call")],
        redaction: Symbol::new("observations-only"),
        replay: Symbol::new("content-addressed-effects"),
    }
}

#[allow(dead_code)]
fn _response_is_public(_: &ModelResponse) {}
