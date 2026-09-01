use super::shared::agent_connection_for_value;
use super::topology_pipeline_sites::{MarketRouteSite, SpeculateSite, StarSite, VerifySite};
use super::topology_runtime::{local_connection, until_value};
use super::topology_sites::{DebateJudgeSite, DebateTurnSite, MeshRoundSite, RingTurnSite};
use crate::installed_codecs;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{Connection, FabricEvalSite, ServerAddress};
use sim_lib_topology::{
    Budget, Edge, Graph, Node, PortRef, TopologyBindingDescriptor, TopologyBindings,
    connection_from_graph_with_bindings,
};
use std::sync::Arc;

struct BoundTarget {
    symbol: Symbol,
    value: Value,
}

pub(crate) fn build_ring_data_graph_connection(
    cx: &mut Cx,
    agents: Vec<Value>,
    role_cycle: Vec<Symbol>,
    max_turns: u32,
) -> Result<Arc<Connection>> {
    if agents.is_empty() {
        return Err(Error::Eval(
            "topology/ring requires at least one agent".to_owned(),
        ));
    }
    let target = local_connection(
        cx,
        Arc::new(RingTurnSite {
            agents: agents
                .into_iter()
                .map(agent_connection_for_value)
                .collect::<Result<Vec<_>>>()?,
            role_cycle,
            max_turns,
        }),
        None,
    )?;
    let target = bind_connection_target(cx, "ring", "turn", target)?;
    let done_value = until_value(cx)?;
    let done = bind_value_target("ring", "done", done_value);
    let graph = loop_graph(
        "agent-topology-ring",
        target.symbol.clone(),
        done.symbol.clone(),
        max_turns,
    );
    build_graph_connection(cx, graph, vec![target, done])
}

pub(crate) fn build_star_data_graph_connection(
    cx: &mut Cx,
    hub: Value,
    spokes: Vec<Value>,
    hub_role: Symbol,
    spoke_role: Symbol,
) -> Result<Arc<Connection>> {
    if spokes.is_empty() {
        return Err(Error::Eval(
            "topology/star requires at least one spoke".to_owned(),
        ));
    }
    let target = local_connection(
        cx,
        Arc::new(StarSite {
            hub: agent_connection_for_value(hub)?,
            spokes: spokes
                .into_iter()
                .map(agent_connection_for_value)
                .collect::<Result<Vec<_>>>()?,
            hub_role,
            spoke_role,
        }),
        None,
    )?;
    let target = bind_connection_target(cx, "star", "stage", target)?;
    build_graph_connection(
        cx,
        call_graph("agent-topology-star", target.symbol.clone()),
        vec![target],
    )
}

pub(crate) fn build_mesh_data_graph_connection(
    cx: &mut Cx,
    agents: Vec<Value>,
    judge: Value,
    max_rounds: u32,
) -> Result<Arc<Connection>> {
    if agents.is_empty() {
        return Err(Error::Eval(
            "topology/mesh requires at least one agent".to_owned(),
        ));
    }
    let target = local_connection(
        cx,
        Arc::new(MeshRoundSite {
            agents: agents
                .into_iter()
                .map(agent_connection_for_value)
                .collect::<Result<Vec<_>>>()?,
            judge,
            max_rounds,
        }),
        None,
    )?;
    let target = bind_connection_target(cx, "mesh", "round", target)?;
    let done_value = until_value(cx)?;
    let done = bind_value_target("mesh", "done", done_value);
    let graph = loop_graph(
        "agent-topology-mesh",
        target.symbol.clone(),
        done.symbol.clone(),
        max_rounds,
    );
    build_graph_connection(cx, graph, vec![target, done])
}

pub(crate) fn build_market_data_graph_connection(
    cx: &mut Cx,
    workers: Vec<Value>,
    router: Value,
) -> Result<Arc<Connection>> {
    if workers.is_empty() {
        return Err(Error::Eval(
            "topology/market requires at least one worker".to_owned(),
        ));
    }
    let target = local_connection(cx, Arc::new(MarketRouteSite { workers, router }), None)?;
    let target = bind_connection_target(cx, "market", "route", target)?;
    build_graph_connection(
        cx,
        call_graph("agent-topology-market", target.symbol.clone()),
        vec![target],
    )
}

pub(crate) fn build_debate_data_graph_connection(
    cx: &mut Cx,
    pro: Value,
    con: Value,
    judge: Value,
    rounds: u32,
) -> Result<Arc<Connection>> {
    let turn = local_connection(
        cx,
        Arc::new(DebateTurnSite {
            pro: agent_connection_for_value(pro)?,
            con: agent_connection_for_value(con)?,
            max_turns: rounds.saturating_mul(2),
        }),
        None,
    )?;
    let judge = local_connection(cx, Arc::new(DebateJudgeSite { judge }), None)?;
    let turn = bind_connection_target(cx, "debate", "turn", turn)?;
    let judge = bind_connection_target(cx, "debate", "judge", judge)?;
    let done_value = until_value(cx)?;
    let done = bind_value_target("debate", "done", done_value);
    let graph = debate_graph(
        "agent-topology-debate",
        turn.symbol.clone(),
        judge.symbol.clone(),
        done.symbol.clone(),
        rounds.saturating_mul(2),
    );
    build_graph_connection(cx, graph, vec![turn, judge, done])
}

pub(crate) fn build_speculate_verify_data_graph_connection(
    cx: &mut Cx,
    speculator: Value,
    verifier: Value,
    on_mismatch: Symbol,
) -> Result<Arc<Connection>> {
    let speculator = local_connection(
        cx,
        Arc::new(SpeculateSite {
            speculator: agent_connection_for_value(speculator)?,
        }),
        Some(Symbol::new("worker")),
    )?;
    let verifier = local_connection(
        cx,
        Arc::new(VerifySite {
            verifier: agent_connection_for_value(verifier)?,
            on_mismatch,
        }),
        Some(Symbol::new("verifier")),
    )?;
    let speculator = bind_connection_target(cx, "speculate-verify", "speculate", speculator)?;
    let verifier = bind_connection_target(cx, "speculate-verify", "verify", verifier)?;
    let graph = pipeline_graph(
        "agent-topology-speculate-verify",
        &[speculator.symbol.clone(), verifier.symbol.clone()],
    );
    build_graph_connection(cx, graph, vec![speculator, verifier])
}

pub(crate) fn build_open_claw_data_graph_connection(
    cx: &mut Cx,
    steps: Vec<Value>,
) -> Result<Arc<Connection>> {
    if steps.is_empty() {
        return Err(Error::Eval(
            "topology/open-claw requires at least one step".to_owned(),
        ));
    }
    let targets = steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            let connection = agent_connection_for_value(step)?;
            bind_connection_target(cx, "open-claw", &format!("step-{index}"), connection)
        })
        .collect::<Result<Vec<_>>>()?;
    let symbols = targets
        .iter()
        .map(|target| target.symbol.clone())
        .collect::<Vec<_>>();
    build_graph_connection(
        cx,
        pipeline_graph("agent-topology-open-claw", &symbols),
        targets,
    )
}

fn build_graph_connection(
    cx: &mut Cx,
    graph: Graph,
    targets: Vec<BoundTarget>,
) -> Result<Arc<Connection>> {
    let mut bindings = TopologyBindings::new();
    for node in &graph.nodes {
        let matching = targets
            .iter()
            .filter(|target| {
                node.target
                    .iter()
                    .chain(node.options.iter().map(|(_, value)| value))
                    .any(|value| matches!(value, Expr::Symbol(symbol) if symbol == &target.symbol))
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(Error::Eval(format!(
                "topology node {} requires more than one live binding",
                node.id.as_symbol()
            )));
        }
        if let Some(target) = matching.first() {
            bindings.bind(
                node.id.clone(),
                TopologyBindingDescriptor::for_node(target.symbol.to_string(), node),
                target.value.clone(),
            );
        }
    }
    let topology = Arc::new(connection_from_graph_with_bindings(cx, &graph, bindings)?);
    let site = FabricEvalSite::new(
        "topology",
        ServerAddress::Local,
        installed_codecs(cx),
        topology,
    );
    Ok(Arc::new(local_connection(cx, Arc::new(site), None)?))
}

fn call_graph(name: &str, target: Symbol) -> Graph {
    pipeline_graph(name, &[target])
}

fn pipeline_graph(name: &str, targets: &[Symbol]) -> Graph {
    let mut graph = Graph::minimal(name);
    let mut nodes = Vec::with_capacity(targets.len() + 2);
    nodes.push(Node::named("in", "in"));
    for (index, target) in targets.iter().enumerate() {
        nodes.push(call_node(&format!("step-{index}"), target.clone()));
    }
    nodes.push(Node::named("out", "out"));

    let mut edges = Vec::with_capacity(targets.len() + 1);
    if targets.is_empty() {
        edges.push(Edge::new(0, PortRef::output("in"), PortRef::input("out")));
    } else {
        edges.push(Edge::new(
            0,
            PortRef::output("in"),
            PortRef::input("step-0"),
        ));
        for index in 0..targets.len().saturating_sub(1) {
            edges.push(Edge::new(
                u32::try_from(index + 1).unwrap_or(u32::MAX),
                PortRef::output(format!("step-{index}")),
                PortRef::input(format!("step-{}", index + 1)),
            ));
        }
        edges.push(Edge::new(
            u32::try_from(targets.len()).unwrap_or(u32::MAX),
            PortRef::output(format!("step-{}", targets.len() - 1)),
            PortRef::input("out"),
        ));
    }

    graph.nodes = nodes;
    graph.edges = edges;
    graph
}

fn loop_graph(name: &str, target: Symbol, done: Symbol, max_visits: u32) -> Graph {
    let mut graph = Graph::minimal(name);
    let mut gate = Node::named("done", "branch");
    gate.options
        .push((Symbol::new("when"), Expr::Symbol(done.clone())));
    let mut back = Edge::new(2, PortRef::named("done", "false"), PortRef::input("step"));
    back.max_visits = Some(max_visits.max(1));
    graph.nodes = vec![
        Node::named("in", "in"),
        call_node("step", target),
        gate,
        Node::named("out", "out"),
    ];
    graph.edges = vec![
        Edge::new(0, PortRef::output("in"), PortRef::input("step")),
        Edge::new(1, PortRef::output("step"), PortRef::input("done")),
        back,
        Edge::new(3, PortRef::named("done", "true"), PortRef::input("out")),
    ];
    graph.budget = Budget {
        max_steps: max_visits.saturating_mul(4).max(16),
        max_node_visits: max_visits.saturating_add(4).max(8),
        max_edge_visits: max_visits.saturating_add(4).max(8),
        ..Budget::default()
    };
    graph
}

fn debate_graph(name: &str, turn: Symbol, judge: Symbol, done: Symbol, max_turns: u32) -> Graph {
    let mut graph = loop_graph(name, turn, done, max_turns);
    graph.nodes.pop();
    graph.nodes.push(call_node("judge", judge));
    graph.nodes.push(Node::named("out", "out"));
    graph.edges.pop();
    graph.edges.push(Edge::new(
        3,
        PortRef::named("done", "true"),
        PortRef::input("judge"),
    ));
    graph.edges.push(Edge::new(
        4,
        PortRef::output("judge"),
        PortRef::input("out"),
    ));
    graph
}

fn call_node(id: &str, target: Symbol) -> Node {
    let mut node = Node::named(id, "call");
    node.target = Some(Expr::Symbol(target));
    node
}

fn bind_connection_target(
    cx: &mut Cx,
    graph: &str,
    name: &str,
    connection: Connection,
) -> Result<BoundTarget> {
    let value = cx.factory().opaque(Arc::new(connection))?;
    Ok(bind_value_target(graph, name, value))
}

fn bind_value_target(graph: &str, name: &str, value: Value) -> BoundTarget {
    BoundTarget {
        symbol: target_symbol(graph, name),
        value,
    }
}

fn target_symbol(graph: &str, name: &str) -> Symbol {
    Symbol::qualified("agent.topology.binding", format!("{graph}/{name}"))
}
