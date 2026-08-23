#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Agent-conduct certification and topology execution adapter.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use sim_kernel::{
    CapabilityName, Cx, DefaultFactory, Error, Expr, NoopEvalPolicy, Result, Symbol, Value,
};
use sim_lib_agent_conduct_core::{AgentConductContract, AgentStepCard};
use sim_lib_topology::{
    CompiledGraph, NodeId, TopologyBindingDescriptor, TopologyBindings, TopologyContinuation,
    TopologyEntry, TopologyPackage, TopologyProgress, TopologyRunReport, compile_graph,
    topology_reflect, topology_reflect_graph,
};

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

impl AgentConduct {
    fn compile(&self, cx: &mut Cx) -> Result<CompiledGraph> {
        compile_graph(cx, &self.topology.graph)
    }

    /// Runs through the topology engine and translates its public output envelope unchanged.
    pub fn run(&self, cx: &mut Cx, frame: Expr, bindings: TopologyBindings) -> Result<Expr> {
        let plan = self.compile(cx)?;
        let mut run = sim_lib_topology::run::TopologyRun::new(&self.topology.graph, &plan, frame)?;
        run.set_bindings(bindings);
        run.run(cx)?;
        Ok(run.output_expr())
    }

    /// Advances exactly one topology work item, optionally resuming a continuation.
    pub fn step(
        &self,
        cx: &mut Cx,
        frame: Expr,
        continuation: Option<TopologyContinuation>,
        bindings: TopologyBindings,
    ) -> Result<AgentConductProgress> {
        let plan = self.compile(cx)?;
        let mut run = match continuation {
            Some(saved) => sim_lib_topology::run::TopologyRun::resume(
                &self.topology.graph,
                &plan,
                saved,
                bindings,
            )?,
            None => {
                let mut run =
                    sim_lib_topology::run::TopologyRun::new(&self.topology.graph, &plan, frame)?;
                run.set_bindings(bindings);
                run
            }
        };
        let progress = run.step(cx)?;
        Ok(AgentConductProgress {
            progress,
            continuation: run.continuation(),
            output: run.output_expr(),
        })
    }

    /// Reflects graph structure through the topology reflection policy.
    pub fn reflect(&self, cx: &Cx) -> Expr {
        topology_reflect_graph(cx, &self.topology.graph)
    }

    /// Runs and reports through the topology reporting implementation.
    pub fn report(&self, cx: &mut Cx, frame: Expr) -> Result<TopologyRunReport> {
        let plan = self.compile(cx)?;
        topology_reflect(cx, &self.topology.graph, &plan, frame)
    }

    /// Returns the topology-owned canonical graph projection used by diagram clients.
    pub fn diagram(&self, cx: &Cx) -> Expr {
        topology_reflect_graph(cx, &self.topology.graph)
    }
}

fn validate_metadata(package: &TopologyPackage) -> Result<()> {
    match metadata(package, "profile") {
        Some(Expr::Symbol(value)) if value.to_string() == AGENT_CONDUCT_PROFILE => Ok(()),
        _ => Err(fail(format!(
            "metadata profile must be {AGENT_CONDUCT_PROFILE}"
        ))),
    }
}
fn validate_boundary(package: &TopologyPackage) -> Result<()> {
    let inputs_ok = package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "in")
        .all(|node| node.output.as_ref() == Some(&run_frame_shape()));
    let outputs_ok = package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "out")
        .all(|node| node.input.as_ref() == Some(&completed_or_stopped_shape()));
    if !inputs_ok {
        return Err(fail("public input Shape must be agent/RunFrame"));
    }
    if !outputs_ok {
        return Err(fail(
            "public output Shape must be completed-or-stopped AgentRunFrame",
        ));
    }
    Ok(())
}
fn card_index(cards: &[AgentStepCard]) -> Result<BTreeMap<Symbol, AgentStepCard>> {
    let mut out = BTreeMap::new();
    for card in cards {
        if out.insert(card.step_id.clone(), card.clone()).is_some() {
            return Err(fail(format!("duplicate AgentStepCard {}", card.step_id)));
        }
    }
    Ok(out)
}
fn validate_calls_and_routes(
    package: &TopologyPackage,
    cards: &BTreeMap<Symbol, AgentStepCard>,
) -> Result<Vec<AgentStepCard>> {
    let mut selected = Vec::new();
    for node in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "call")
    {
        let target = target_symbol(node).ok_or_else(|| {
            fail(format!(
                "call node {} target must be a step symbol",
                node.id.as_symbol()
            ))
        })?;
        let card = cards.get(target).ok_or_else(|| {
            fail(format!(
                "call target {target} has no registered AgentStepCard"
            ))
        })?;
        if card.input_shape != run_frame_shape() || card.output_shape != run_frame_shape() {
            return Err(fail(format!(
                "step Card {target} is not RunFrame-compatible"
            )));
        }
        let outgoing: Vec<_> = package
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from.node == node.id)
            .collect();
        for outcome in &card.outcomes {
            if is_terminal(target) {
                break;
            }
            let predicate = outcome_predicate(outcome);
            let count = outgoing
                .iter()
                .filter(|edge| edge.when.as_ref() == Some(&predicate))
                .count();
            if count != 1 {
                return Err(fail(format!(
                    "call target {target} outcome {outcome} requires exactly one outgoing predicate route, found {count}"
                )));
            }
        }
        selected.push(card.clone());
    }
    Ok(selected)
}
fn validate_terminal_admission(package: &TopologyPackage) -> Result<()> {
    let nodes: BTreeMap<_, _> = package
        .graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect();
    for out in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "out")
    {
        for edge in package
            .graph
            .edges
            .iter()
            .filter(|edge| edge.to.node == out.id)
        {
            let source = nodes
                .get(&edge.from.node)
                .ok_or_else(|| fail("route target is not reachable"))?;
            if !target_symbol(source).is_some_and(is_terminal) {
                return Err(fail(format!(
                    "public output {} is not admitted by finish or stop",
                    out.id.as_symbol()
                )));
            }
        }
    }
    Ok(())
}
fn derive_roles(package: &TopologyPackage, cards: &[AgentStepCard]) -> Result<Vec<Symbol>> {
    let mut roles = BTreeSet::new();
    for node in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "call")
    {
        if let Some(role) = &node.role {
            roles.insert(role.clone());
        }
    }
    for card in cards {
        roles.extend(card.roles.iter().cloned());
    }
    Ok(roles.into_iter().collect())
}
fn validate_declared_roles(package: &TopologyPackage, roles: &[Symbol]) -> Result<()> {
    let Some(value) = metadata(package, "requires-roles") else {
        return Ok(());
    };
    let Expr::List(items) = value else {
        return Err(fail("requires-roles metadata must be a symbol list"));
    };
    let mut declared = BTreeSet::new();
    for item in items {
        let Expr::Symbol(role) = item else {
            return Err(fail("requires-roles metadata must contain symbols"));
        };
        declared.insert(role.clone());
    }
    if declared != roles.iter().cloned().collect() {
        return Err(fail(
            "requires-roles metadata disagrees with roles derived from nodes and Cards",
        ));
    }
    Ok(())
}
fn derive_capabilities(package: &TopologyPackage, cards: &[AgentStepCard]) -> Vec<CapabilityName> {
    let mut out: BTreeSet<_> = package.capabilities.iter().cloned().collect();
    for card in cards {
        out.extend(card.capabilities.iter().cloned());
    }
    out.into_iter().collect()
}
fn metadata<'a>(package: &'a TopologyPackage, name: &str) -> Option<&'a Expr> {
    let normalized = name.replace('-', "_");
    package
        .metadata
        .iter()
        .find(|(key, _)| key.name.as_ref() == normalized)
        .map(|(_, value)| value)
}
fn target_symbol(node: &sim_lib_topology::Node) -> Option<&Symbol> {
    match &node.target {
        Some(Expr::Symbol(value)) => Some(value),
        _ => None,
    }
}
fn is_terminal(target: &Symbol) -> bool {
    target.namespace.as_deref() == Some("agent.step")
        && matches!(target.name.as_ref(), "finish" | "stop")
}
fn outcome_predicate(outcome: &Symbol) -> Expr {
    Expr::Symbol(Symbol::qualified(
        "agent",
        format!("outcome-{}", outcome.name),
    ))
}
fn card_expr(card: &AgentStepCard) -> Expr {
    Expr::Symbol(card.step_id.clone())
}
fn fail(message: impl Into<String>) -> Error {
    Error::Eval(format!("agent conduct: {}", message.into()))
}

#[cfg(test)]
mod tests;
