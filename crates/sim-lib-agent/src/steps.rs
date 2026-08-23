//! Reusable, single-attempt agent model and tool step boundaries.

use std::{collections::BTreeMap, sync::Arc};

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
use crate::{Component, ComponentKind, PlanningOutput, PlanningTask, planning};

/// A callable step produced by an [`AgentStepFactory`].
pub trait AgentStep: Send + Sync {
    /// Executes exactly one attempt and returns its redacted event.
    fn execute(&self, cx: &mut Cx, frame: &mut AgentRunFrame) -> Result<AgentEvent>;
}

/// Open factory contract used by hosts to add conduct steps without a central enum.
pub trait AgentStepFactory: Send + Sync {
    /// Version of the Card contract implemented by this factory.
    fn version(&self) -> u64;
    /// Binds immutable node options and roles into one callable step.
    fn bind(&self, roles: &BTreeMap<Symbol, Expr>, options: &Expr) -> Result<Arc<dyn AgentStep>>;
}

/// Card and factory registry used when binding a conduct package.
#[derive(Default)]
pub struct AgentStepRegistry {
    entries: BTreeMap<Symbol, (AgentStepCard, Arc<dyn AgentStepFactory>)>,
}

impl AgentStepRegistry {
    /// Creates an empty, host-owned registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one extension, rejecting duplicate ids and version disagreement.
    pub fn register(
        &mut self,
        card: AgentStepCard,
        factory: Arc<dyn AgentStepFactory>,
    ) -> Result<()> {
        if self.entries.contains_key(&card.step_id) {
            return Err(sim_kernel::Error::Eval(format!(
                "duplicate agent step {}",
                card.step_id
            )));
        }
        if card.version != factory.version() {
            return Err(sim_kernel::Error::Eval(format!(
                "agent step {} Card version {} does not match factory version {}",
                card.step_id,
                card.version,
                factory.version()
            )));
        }
        self.entries.insert(card.step_id.clone(), (card, factory));
        Ok(())
    }

    /// Returns the registered Card for an exact id.
    pub fn card(&self, id: &Symbol) -> Option<&AgentStepCard> {
        self.entries.get(id).map(|(card, _)| card)
    }

    /// Binds one registered factory to immutable roles and node options.
    pub fn bind(
        &self,
        id: &Symbol,
        roles: &BTreeMap<Symbol, Expr>,
        options: &Expr,
    ) -> Result<Arc<dyn AgentStep>> {
        let (_, factory) = self
            .entries
            .get(id)
            .ok_or_else(|| sim_kernel::Error::Eval(format!("unregistered agent step {id}")))?;
        factory.bind(roles, options)
    }

    /// Returns all Cards in stable id order for conduct certification.
    pub fn cards(&self) -> Vec<AgentStepCard> {
        self.entries
            .values()
            .map(|(card, _)| card.clone())
            .collect()
    }
}

/// Immutable phase declaration captured from normal node options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseOptions {
    /// Stable phase id.
    pub id: Symbol,
    /// Instructions supplied to the normal model or component step.
    pub instructions: Expr,
    /// Exact tool role ids admitted during this phase.
    pub allowed_tools: Vec<Symbol>,
}

/// Result of a single review attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewOutcome {
    /// The candidate is admitted.
    Accept,
    /// The graph should route to a revision step.
    Revise,
    /// The candidate is refused.
    Reject,
}

/// Stable shaped result returned from one delegated child run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegatedObservation {
    /// Parent/child correlation id.
    pub correlation: Symbol,
    /// Child output converted back to an expression.
    pub output: Expr,
    /// Usage charged to the parent.
    pub charge: UsageQuantity,
}

fn event(kind: &str, fields: Vec<(Expr, Expr)>) -> AgentEvent {
    AgentEvent::new(Symbol::qualified("agent.event", kind), Expr::Map(fields))
}

fn field(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

/// Calls exactly one bound component and records reference-only provenance.
pub fn execute_component_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    role: Symbol,
    component: &dyn Component,
    request: EvalRequest,
    input_reference: Expr,
) -> Result<AgentEvent> {
    if !matches!(
        component.kind(),
        ComponentKind::Router
            | ComponentKind::Retriever
            | ComponentKind::Sandbox
            | ComponentKind::Recorder
            | ComponentKind::Persona
            | ComponentKind::Voice
            | ComponentKind::Custom(_)
    ) {
        return Err(sim_kernel::Error::Eval(format!(
            "component {} is not a component-step role",
            component.name()
        )));
    }
    let fabric = component.as_eval_fabric().ok_or_else(|| {
        sim_kernel::Error::Eval(format!(
            "component {} has no EvalFabric projection",
            component.name()
        ))
    })?;
    let reply = fabric.realize(cx, request)?;
    let output = reply.value.object().as_expr(cx)?;
    frame.working = output.clone();
    frame.outcome = Symbol::new("continue");
    Ok(event(
        "component",
        vec![
            field("role", Expr::Symbol(role)),
            field("component", Expr::Symbol(component.name().clone())),
            field("input-reference", input_reference),
            field("output-reference", output),
            field("outcome", Expr::Symbol(frame.outcome.clone())),
        ],
    ))
}

/// Runs the real planning decomposition once and stores the resulting plan in the frame.
pub fn execute_plan_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    goal: &PlanningTask,
    runner: &dyn ModelRunner,
    max_steps: u32,
) -> Result<AgentEvent> {
    match planning::decompose(cx, goal, runner, max_steps) {
        Ok(tasks) => {
            let plan = Expr::List(
                tasks
                    .iter()
                    .map(|task| {
                        Expr::Map(vec![
                            field("id", Expr::String(task.id.clone())),
                            field("prompt", Expr::String(task.prompt.clone())),
                        ])
                    })
                    .collect(),
            );
            frame.state.upsert(Symbol::new("plan"), plan)?;
            frame.outcome = Symbol::new("created");
        }
        Err(error) => {
            frame.outcome = Symbol::new("error");
            return Err(error);
        }
    }
    Ok(event(
        "plan",
        vec![field("outcome", Expr::Symbol(frame.outcome.clone()))],
    ))
}

/// Runs the real reflection combinator once and returns an open graph-owned replan outcome.
pub fn execute_replan_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    output: &PlanningOutput,
    runner: &dyn ModelRunner,
) -> Result<AgentEvent> {
    let reflected = planning::reflect(cx, output, runner, 0)?;
    frame.outcome = if reflected.accept {
        Symbol::new("keep")
    } else if reflected.retry.is_some() {
        Symbol::new("replace")
    } else if reflected.critique.to_ascii_lowercase().starts_with("stop") {
        Symbol::new("stop")
    } else if reflected.critique.to_ascii_lowercase().starts_with("done") {
        Symbol::new("done")
    } else {
        Symbol::new("replace")
    };
    Ok(event(
        "replan",
        vec![field("outcome", Expr::Symbol(frame.outcome.clone()))],
    ))
}

/// Records entry into a phase declared by ordinary node options.
pub fn enter_phase(frame: &mut AgentRunFrame, phase: &PhaseOptions) -> Result<AgentEvent> {
    frame
        .state
        .upsert(Symbol::new("phase"), Expr::Symbol(phase.id.clone()))?;
    frame.state.upsert(
        Symbol::qualified("agent.phase", "instructions"),
        phase.instructions.clone(),
    )?;
    frame.state.upsert(
        Symbol::qualified("agent.phase", "allowed-tools"),
        Expr::List(
            phase
                .allowed_tools
                .iter()
                .cloned()
                .map(Expr::Symbol)
                .collect(),
        ),
    )?;
    Ok(event(
        "phase-entered",
        vec![field("phase", Expr::Symbol(phase.id.clone()))],
    ))
}

/// Enforces the phase's declared subset before a normal tool step is invoked.
pub fn admit_phase_tool(phase: &PhaseOptions, tool: &Symbol) -> Result<()> {
    if phase.allowed_tools.contains(tool) {
        Ok(())
    } else {
        Err(sim_kernel::Error::Eval(format!(
            "tool {tool} is outside phase {} allowed subset",
            phase.id
        )))
    }
}

/// Records completion of the currently entered phase.
pub fn complete_phase(frame: &mut AgentRunFrame, phase: &PhaseOptions) -> Result<AgentEvent> {
    frame.state.upsert(Symbol::new("phase"), Expr::Nil)?;
    Ok(event(
        "phase-completed",
        vec![field("phase", Expr::Symbol(phase.id.clone()))],
    ))
}

/// Performs one review through the real reflection boundary and never loops.
pub fn execute_review_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    candidate: &PlanningOutput,
    reviewer: &dyn ModelRunner,
) -> Result<ReviewOutcome> {
    let result = planning::reflect(cx, candidate, reviewer, 0)?;
    let outcome = if result.accept {
        ReviewOutcome::Accept
    } else if result.critique.to_ascii_lowercase().starts_with("reject") {
        ReviewOutcome::Reject
    } else {
        ReviewOutcome::Revise
    };
    frame.outcome = Symbol::new(match outcome {
        ReviewOutcome::Accept => "accept",
        ReviewOutcome::Revise => "revise",
        ReviewOutcome::Reject => "reject",
    });
    Ok(outcome)
}

/// Admits the result against the intersection of conduct, caller, and component Shapes.
pub fn execute_finish(
    frame: &mut AgentRunFrame,
    cx: &mut Cx,
    shapes: &[Expr],
) -> Result<AgentEvent> {
    for shape in shapes {
        let parsed = sim_shape::parse_shape_expr(shape)?;
        let matched = sim_shape::check_shape_on_expr(parsed.as_ref(), cx, &frame.working)?;
        if !matched.accepted {
            return Err(sim_shape::shape_error(parsed.as_ref(), cx, &frame.working)?);
        }
    }
    frame.outcome = Symbol::new("finished");
    Ok(event(
        "finish",
        vec![field("outcome", Expr::Symbol(frame.outcome.clone()))],
    ))
}

/// Stops with a stable code and redacted evidence retained in the frame.
pub fn execute_stop(frame: &mut AgentRunFrame, code: Symbol, evidence: Expr) -> Result<AgentEvent> {
    frame.outcome = code.clone();
    frame.state.upsert(
        Symbol::qualified("agent.stop", "evidence"),
        evidence.clone(),
    )?;
    Ok(event(
        "stop",
        vec![
            field("code", Expr::Symbol(code)),
            field("evidence", evidence),
        ],
    ))
}

/// Delegates one child request through an existing fabric with diminished authority.
pub fn execute_delegate_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    fabric: &dyn EvalFabric,
    correlation: Symbol,
    allowed: sim_kernel::CapabilitySet,
    charge: UsageQuantity,
    budget: &AgentUsageBudget,
    request: EvalRequest,
) -> Result<DelegatedObservation> {
    if budget.admit(&frame.usage, &charge).is_err() {
        return Err(sim_kernel::Error::Eval("child run budget exhausted".into()));
    }
    let diminished = sim_kernel::capability::diminish(cx.capabilities(), &allowed);
    let reply = cx.with_capabilities(diminished, |cx| fabric.realize(cx, request))?;
    frame
        .usage
        .charge(budget, charge.clone())
        .map_err(|error| sim_kernel::Error::Eval(error.to_string()))?;
    let output = reply.value.object().as_expr(cx)?;
    frame.working = output.clone();
    frame.state.upsert(
        Symbol::qualified("agent.delegate", "correlation"),
        Expr::Symbol(correlation.clone()),
    )?;
    Ok(DelegatedObservation {
        correlation,
        output,
        charge,
    })
}

/// Returns an explicit policy-requested yield event; it performs no lifecycle journaling.
pub fn execute_checkpoint(frame: &mut AgentRunFrame, reason: Expr) -> Result<AgentEvent> {
    frame.outcome = Symbol::new("checkpoint");
    Ok(event("checkpoint", vec![field("reason", reason)]))
}

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
        /// Checked reply packet accepted as the final model result.
        packet: BridgePacket,
        /// Redacted event recorded for this model turn.
        event: AgentEvent,
    },
    /// A checked reply asks the graph to execute tool calls next.
    ToolCalls {
        /// Checked reply packet carrying the requested tool calls.
        packet: BridgePacket,
        /// Redacted event recorded for this model turn.
        event: AgentEvent,
    },
    /// The reply requires a later graph-owned repair step.
    RepairNeeded {
        /// Bounded ASK failure that the graph can route to repair.
        failure: sim_lib_bridge::AskFailure,
        /// Redacted event recorded for this model turn.
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

/// Cards for every standard conduct step, in stable id order.
pub fn standard_step_cards() -> Vec<AgentStepCard> {
    let frame = run_frame_shape();
    let specs: &[(&str, &[&str], &[&str], bool, &[&str], &str)] = &[
        (
            "checkpoint",
            &[],
            &["checkpoint"],
            false,
            &[],
            "deterministic",
        ),
        (
            "component",
            &["component"],
            &["continue", "error"],
            true,
            &["component-call"],
            "effect-safe",
        ),
        (
            "delegate",
            &["delegate"],
            &["continue", "error"],
            true,
            &["child-run"],
            "content-addressed-effects",
        ),
        (
            "finish",
            &[],
            &["finished", "error"],
            false,
            &[],
            "deterministic",
        ),
        (
            "model-turn",
            &["runner"],
            &["tool-calls", "final", "error"],
            true,
            &["model-turn"],
            "effect-safe",
        ),
        (
            "plan",
            &["runner"],
            &["created", "error"],
            true,
            &["model-turn"],
            "effect-safe",
        ),
        (
            "replan",
            &["runner"],
            &["keep", "replace", "done", "stop", "error"],
            true,
            &["model-turn"],
            "effect-safe",
        ),
        (
            "review",
            &["reviewer"],
            &["accept", "revise", "reject", "error"],
            true,
            &["model-turn"],
            "effect-safe",
        ),
        ("stop", &[], &["stopped"], false, &[], "deterministic"),
        (
            "tool-batch",
            &["tools"],
            &["continue", "final", "error"],
            true,
            &["tool-call"],
            "content-addressed-effects",
        ),
    ];
    specs
        .iter()
        .map(
            |(id, roles, outcomes, effect, usage, replay)| AgentStepCard {
                step_id: Symbol::qualified("agent.step", *id),
                version: 1,
                input_shape: frame.clone(),
                output_shape: frame.clone(),
                roles: roles.iter().map(|role| Symbol::new(*role)).collect(),
                capabilities: vec![],
                outcomes: outcomes
                    .iter()
                    .map(|outcome| Symbol::new(*outcome))
                    .collect(),
                may_request_effect: *effect,
                usage_dimensions: usage
                    .iter()
                    .map(|dimension| Symbol::qualified("agent.usage", *dimension))
                    .collect(),
                redaction: Symbol::new("references-only"),
                replay: Symbol::new(*replay),
            },
        )
        .collect()
}

#[allow(dead_code)]
fn _response_is_public(_: &ModelResponse) {}
