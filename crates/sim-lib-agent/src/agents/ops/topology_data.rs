use super::shared::agent_connection_for_value;
use super::topology_pipeline_sites::{MarketRouteSite, SpeculateSite, StarSite, VerifySite};
use super::topology_runtime::{
    DebateSession, MeshSession, RingSession, local_connection, until_value,
};
use super::topology_sites::{DebateJudgeSite, DebateTurnSite, MeshRoundSite, RingTurnSite};
use crate::installed_codecs;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{Connection, FabricEvalSite, ServerAddress};
use sim_lib_topology::{Budget, Edge, Graph, Node, PortRef, connection_from_graph};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static DATA_TARGET_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    let session = Arc::new(Mutex::new(RingSession {
        current: Expr::Nil,
        transcript: Vec::new(),
        agent_index: 0,
        role_index: 0,
        turns_used: 0,
        max_turns,
        done: false,
    }));
    let target = local_connection(
        cx,
        Arc::new(RingTurnSite {
            session,
            agents: agents
                .into_iter()
                .map(agent_connection_for_value)
                .collect::<Result<Vec<_>>>()?,
            role_cycle,
        }),
        None,
    )?;
    let target = register_connection_target(cx, "ring", "turn", target)?;
    let done_value = until_value(cx)?;
    let done = register_value_target(cx, "ring", "done", done_value)?;
    build_graph_connection(
        cx,
        loop_graph("agent-topology-ring", target, done, max_turns),
    )
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
    let target = register_connection_target(cx, "star", "stage", target)?;
    build_graph_connection(cx, call_graph("agent-topology-star", target))
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
    let session = Arc::new(Mutex::new(MeshSession {
        candidate: Expr::Nil,
        transcript: Vec::new(),
        rounds_used: 0,
        max_rounds,
        best_score: None,
        done: false,
    }));
    let target = local_connection(
        cx,
        Arc::new(MeshRoundSite {
            session,
            agents: agents
                .into_iter()
                .map(agent_connection_for_value)
                .collect::<Result<Vec<_>>>()?,
            judge,
        }),
        None,
    )?;
    let target = register_connection_target(cx, "mesh", "round", target)?;
    let done_value = until_value(cx)?;
    let done = register_value_target(cx, "mesh", "done", done_value)?;
    build_graph_connection(
        cx,
        loop_graph("agent-topology-mesh", target, done, max_rounds),
    )
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
    let target = register_connection_target(cx, "market", "route", target)?;
    build_graph_connection(cx, call_graph("agent-topology-market", target))
}

pub(crate) fn build_debate_data_graph_connection(
    cx: &mut Cx,
    pro: Value,
    con: Value,
    judge: Value,
    rounds: u32,
) -> Result<Arc<Connection>> {
    let session = Arc::new(Mutex::new(DebateSession {
        task: Expr::Nil,
        transcript: Vec::new(),
        turns_used: 0,
        max_turns: rounds.saturating_mul(2),
        pro_turn: true,
        done: false,
    }));
    let turn = local_connection(
        cx,
        Arc::new(DebateTurnSite {
            session: session.clone(),
            pro: agent_connection_for_value(pro)?,
            con: agent_connection_for_value(con)?,
        }),
        None,
    )?;
    let judge = local_connection(cx, Arc::new(DebateJudgeSite { session, judge }), None)?;
    let turn = register_connection_target(cx, "debate", "turn", turn)?;
    let judge = register_connection_target(cx, "debate", "judge", judge)?;
    let done_value = until_value(cx)?;
    let done = register_value_target(cx, "debate", "done", done_value)?;
    build_graph_connection(
        cx,
        debate_graph(
            "agent-topology-debate",
            turn,
            judge,
            done,
            rounds.saturating_mul(2),
        ),
    )
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
    let speculator = register_connection_target(cx, "speculate-verify", "speculate", speculator)?;
    let verifier = register_connection_target(cx, "speculate-verify", "verify", verifier)?;
    build_graph_connection(
        cx,
        pipeline_graph("agent-topology-speculate-verify", &[speculator, verifier]),
    )
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
            register_connection_target(cx, "open-claw", &format!("step-{index}"), connection)
        })
        .collect::<Result<Vec<_>>>()?;
    build_graph_connection(cx, pipeline_graph("agent-topology-open-claw", &targets))
}

fn build_graph_connection(cx: &mut Cx, graph: Graph) -> Result<Arc<Connection>> {
    let topology = Arc::new(connection_from_graph(cx, &graph)?);
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

fn register_connection_target(
    cx: &mut Cx,
    graph: &str,
    name: &str,
    connection: Connection,
) -> Result<Symbol> {
    let value = cx.factory().opaque(Arc::new(connection))?;
    register_value_target(cx, graph, name, value)
}

fn register_value_target(cx: &mut Cx, graph: &str, name: &str, value: Value) -> Result<Symbol> {
    let symbol = target_symbol(graph, name);
    cx.registry_mut().register_value(symbol.clone(), value)?;
    Ok(symbol)
}

fn target_symbol(graph: &str, name: &str) -> Symbol {
    let nonce = DATA_TARGET_COUNTER.fetch_add(1, Ordering::Relaxed);
    Symbol::qualified("topology/data-target", format!("{graph}-{name}-{nonce}"))
}
