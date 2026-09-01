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

/// Immutable inputs for one diminished child delegation.
#[derive(Clone)]
pub struct DelegateRequest {
    /// Stable identity joining the parent state to the child observation.
    pub correlation: Symbol,
    /// Maximum capability set visible during child realization.
    pub allowed: sim_kernel::CapabilitySet,
    /// Usage charged only after successful realization.
    pub charge: UsageQuantity,
    /// Fully specified fabric request.
    pub request: EvalRequest,
}

/// Delegates one child request through an existing fabric with diminished authority.
pub fn execute_delegate_once(
    cx: &mut Cx,
    frame: &mut AgentRunFrame,
    fabric: &dyn EvalFabric,
    budget: &AgentUsageBudget,
    delegation: DelegateRequest,
) -> Result<DelegatedObservation> {
    if budget.admit(&frame.usage, &delegation.charge).is_err() {
        return Err(sim_kernel::Error::Eval("child run budget exhausted".into()));
    }
    let diminished = sim_kernel::capability::diminish(cx.capabilities(), &delegation.allowed);
    let reply = cx.with_capabilities(diminished, |cx| fabric.realize(cx, delegation.request))?;
    frame
        .usage
        .charge(budget, delegation.charge.clone())
        .map_err(|error| sim_kernel::Error::Eval(error.to_string()))?;
    let output = reply.value.object().as_expr(cx)?;
    frame.working = output.clone();
    frame.state.upsert(
        Symbol::qualified("agent.delegate", "correlation"),
        Expr::Symbol(delegation.correlation.clone()),
    )?;
    Ok(DelegatedObservation {
        correlation: delegation.correlation,
        output,
        charge: delegation.charge,
    })
}

/// Returns an explicit policy-requested yield event; it performs no lifecycle journaling.
pub fn execute_checkpoint(frame: &mut AgentRunFrame, reason: Expr) -> Result<AgentEvent> {
    frame.outcome = Symbol::new("checkpoint");
    Ok(event("checkpoint", vec![field("reason", reason)]))
}
