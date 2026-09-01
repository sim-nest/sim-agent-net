/// Required package profile.
pub const AGENT_CONDUCT_PROFILE: &str = "agent/conduct-v1";

/// Standard public input Shape for every conduct.
pub fn run_frame_shape() -> Expr {
    Expr::Symbol(Symbol::qualified("agent", "RunFrame"))
}

/// Standard public output Shape for a completed or stopped conduct.
pub fn completed_or_stopped_shape() -> Expr {
    Expr::Symbol(Symbol::qualified("agent", "RunFrame"))
}

/// Certified projection over one original topology entry.
#[derive(Clone, Debug)]
pub struct AgentConduct {
    /// Original topology entry; this projection owns no graph copy.
    pub topology: TopologyEntry,
    /// Derived data-only conduct contract.
    pub contract: AgentConductContract,
    /// Topology engine fingerprint.
    pub graph_fingerprint: String,
    /// Roles derived from call-node tags and Cards.
    pub required_roles: Vec<Symbol>,
    /// Union of topology and Card capabilities.
    pub capabilities: Vec<CapabilityName>,
    /// Cards selected by call targets.
    pub step_cards: Vec<AgentStepCard>,
    /// Small deterministic summary for browse surfaces.
    pub browse_summary: AgentConductBrowseSummary,
}

/// Stable browse facts derived during certification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConductBrowseSummary {
    /// Package name.
    pub name: Symbol,
    /// Number of call nodes.
    pub call_nodes: usize,
    /// Number of declared routes.
    pub routes: usize,
    /// Terminal admission targets present in the package.
    pub terminal_steps: Vec<Symbol>,
}

/// An explicit live binding for one topology node and one Card identity.
pub struct AgentNodeBinding {
    /// Topology call-node id.
    pub node: NodeId,
    /// Card step id implemented by the value.
    pub step_id: Symbol,
    /// Live callable or otherwise topology-adaptable value.
    pub value: Value,
}

impl AgentNodeBinding {
    /// Creates an explicit node binding.
    pub fn new(node: impl Into<NodeId>, step_id: Symbol, value: Value) -> Self {
        Self {
            node: node.into(),
            step_id,
            value,
        }
    }
}

/// Result of exactly one topology-engine step.
pub struct AgentConductProgress {
    /// Generic topology progress result.
    pub progress: TopologyProgress,
    /// Continuation sealed after that step.
    pub continuation: TopologyContinuation,
    /// Current public output envelope.
    pub output: Expr,
}

/// Certifies one delivered package as an `agent/conduct-v1` conduct.
pub fn validate_agent_conduct(
    package: TopologyPackage,
    step_cards: &[AgentStepCard],
) -> Result<AgentConduct> {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let plan = compile_graph(&mut cx, &package.graph)?;
    validate_metadata(&package)?;
    validate_boundary(&package)?;
    let cards = card_index(step_cards)?;
    let selected = validate_calls_and_routes(&package, &cards)?;
    validate_terminal_admission(&package)?;
    let roles = derive_roles(&package, &selected)?;
    validate_declared_roles(&package, &roles)?;
    let capabilities = derive_capabilities(&package, &selected);
    let fingerprint = sim_lib_topology::run::TopologyRun::new(&package.graph, &plan, Expr::Nil)?
        .continuation()
        .fingerprint()
        .to_owned();
    let entry = TopologyEntry {
        name: package.name().clone(),
        graph: package.graph,
        source: None,
        metadata: package.metadata,
        capabilities: package.capabilities,
    };
    let contract = AgentConductContract {
        profile: Symbol::qualified("agent", "conduct-v1"),
        graph_fingerprint: fingerprint.clone(),
        step_cards: selected.iter().map(card_expr).collect(),
        frame_shape: run_frame_shape(),
    };
    let terminal_steps = entry
        .graph
        .nodes
        .iter()
        .filter_map(|node| {
            target_symbol(node)
                .filter(|target| is_terminal(target))
                .cloned()
        })
        .collect();
    Ok(AgentConduct {
        browse_summary: AgentConductBrowseSummary {
            name: entry.name.clone(),
            call_nodes: selected.len(),
            routes: entry.graph.edges.len(),
            terminal_steps,
        },
        topology: entry,
        contract,
        graph_fingerprint: fingerprint,
        required_roles: roles,
        capabilities,
        step_cards: selected,
    })
}

/// Validates and converts explicit agent bindings to topology bindings.
pub fn bind_agent_conduct(
    conduct: &AgentConduct,
    bindings: Vec<AgentNodeBinding>,
) -> Result<TopologyBindings> {
    let mut seen = BTreeSet::new();
    let mut supplied = BTreeMap::new();
    for binding in bindings {
        if !seen.insert(binding.node.clone()) {
            return Err(fail(format!(
                "duplicate binding for node {}",
                binding.node.as_symbol()
            )));
        }
        supplied.insert(binding.node.clone(), binding);
    }
    let mut topology = TopologyBindings::new();
    for node in conduct
        .topology
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "call")
    {
        let Some(binding) = supplied.remove(&node.id) else {
            return Err(fail(format!(
                "missing binding for call node {}",
                node.id.as_symbol()
            )));
        };
        let expected =
            target_symbol(node).ok_or_else(|| fail("call target must be a step symbol"))?;
        if binding.step_id != *expected {
            return Err(fail(format!(
                "Card-incompatible binding for node {}: expected {}, found {}",
                node.id.as_symbol(),
                expected,
                binding.step_id
            )));
        }
        topology.bind(
            node.id.clone(),
            TopologyBindingDescriptor::for_node(binding.step_id.to_string(), node),
            binding.value,
        );
    }
    if let Some((node, _)) = supplied.first_key_value() {
        return Err(fail(format!(
            "binding names non-call or unknown node {}",
            node.as_symbol()
        )));
    }
    Ok(topology)
}
