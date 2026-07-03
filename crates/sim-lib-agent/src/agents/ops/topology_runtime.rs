use super::shared::agent_connection_for_value;
use super::topology_pipeline_sites::{MarketRouteSite, SpeculateSite, StarSite, VerifySite};
use super::topology_sites::{DebateJudgeSite, DebateTurnSite, MeshRoundSite, RingTurnSite};
use crate::{Agent, AgentComponent, installed_codecs};
use sim_kernel::{
    Args, Callable, ClassRef, Cx, Error, EvalRequest, Expr, NumberLiteral, Object, Result, Symbol,
    Value,
};
use sim_lib_server::{
    Connection, EvalSite, FrameEnvelope, FrameKind, LoopEvalSite, PipelineEvalSite, ServerAddress,
    ServerFrame, eval_reply_from_frame, server_frame_from_request, stream_frame_to_expr,
};
use std::{
    any::Any,
    sync::{Arc, Mutex},
};

pub(super) struct TopologyUntilFn;

#[derive(Clone)]
pub(super) struct RingSession {
    pub(super) current: Expr,
    pub(super) transcript: Vec<Expr>,
    pub(super) agent_index: usize,
    pub(super) role_index: usize,
    pub(super) turns_used: u32,
    pub(super) max_turns: u32,
    pub(super) done: bool,
}

#[derive(Clone)]
pub(super) struct MeshSession {
    pub(super) candidate: Expr,
    pub(super) transcript: Vec<Expr>,
    pub(super) rounds_used: u32,
    pub(super) max_rounds: u32,
    pub(super) best_score: Option<f64>,
    pub(super) done: bool,
}

#[derive(Clone)]
pub(super) struct DebateSession {
    pub(super) task: Expr,
    pub(super) transcript: Vec<Expr>,
    pub(super) turns_used: u32,
    pub(super) max_turns: u32,
    pub(super) pro_turn: bool,
    pub(super) done: bool,
}

pub(super) fn build_ring_connection(
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
    let step = local_connection(
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
    loop_connection(
        cx,
        vec![step],
        usize::try_from(max_turns).unwrap_or(usize::MAX),
    )
}

pub(super) fn build_star_connection(
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
    let step = local_connection(
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
    pipeline_connection(cx, vec![step])
}

pub(super) fn build_mesh_connection(
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
    let step = local_connection(
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
    loop_connection(
        cx,
        vec![step],
        usize::try_from(max_rounds.max(1)).unwrap_or(usize::MAX),
    )
}

pub(super) fn build_market_connection(
    cx: &mut Cx,
    workers: Vec<Value>,
    router: Value,
) -> Result<Arc<Connection>> {
    if workers.is_empty() {
        return Err(Error::Eval(
            "topology/market requires at least one worker".to_owned(),
        ));
    }
    let step = local_connection(cx, Arc::new(MarketRouteSite { workers, router }), None)?;
    pipeline_connection(cx, vec![step])
}

pub(super) fn build_debate_connection(
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
    let loop_step = local_connection(
        cx,
        Arc::new(DebateTurnSite {
            session: session.clone(),
            pro: agent_connection_for_value(pro)?,
            con: agent_connection_for_value(con)?,
        }),
        None,
    )?;
    let until = until_value(cx)?;
    let debate_loop = local_connection(
        cx,
        Arc::new(LoopEvalSite::new(
            pipeline_address(std::slice::from_ref(&loop_step)),
            installed_codecs(cx),
            vec![loop_step],
            usize::try_from(rounds.saturating_mul(2).max(1)).unwrap_or(usize::MAX),
            until,
        )),
        None,
    )?;
    let verdict = local_connection(cx, Arc::new(DebateJudgeSite { session, judge }), None)?;
    pipeline_connection(cx, vec![debate_loop, verdict])
}

pub(super) fn build_speculate_verify_connection(
    cx: &mut Cx,
    speculator: Value,
    verifier: Value,
    on_mismatch: Symbol,
) -> Result<Arc<Connection>> {
    let speculator = agent_connection_for_value(speculator)?;
    let verifier = agent_connection_for_value(verifier)?;
    let steps = vec![
        local_connection(
            cx,
            Arc::new(SpeculateSite { speculator }),
            Some(Symbol::new("worker")),
        )?,
        local_connection(
            cx,
            Arc::new(VerifySite {
                verifier,
                on_mismatch,
            }),
            Some(Symbol::new("verifier")),
        )?,
    ];
    pipeline_connection(cx, steps)
}

pub(super) fn pipeline_connection(cx: &mut Cx, steps: Vec<Connection>) -> Result<Arc<Connection>> {
    let codecs = installed_codecs(cx);
    let address = pipeline_address(&steps);
    Ok(Arc::new(Connection::with_session(
        address.clone(),
        first_codec(&codecs),
        codecs.clone(),
        Arc::new(PipelineEvalSite::new(address, codecs, steps)),
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?))
}

pub(super) fn loop_connection(
    cx: &mut Cx,
    steps: Vec<Connection>,
    max_iterations: usize,
) -> Result<Arc<Connection>> {
    let codecs = installed_codecs(cx);
    let address = pipeline_address(&steps);
    Ok(Arc::new(Connection::with_session(
        address.clone(),
        first_codec(&codecs),
        codecs.clone(),
        Arc::new(LoopEvalSite::new(
            address,
            codecs,
            steps,
            max_iterations,
            until_value(cx)?,
        )),
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?))
}

pub(super) fn local_connection(
    cx: &mut Cx,
    site: Arc<dyn EvalSite>,
    role: Option<Symbol>,
) -> Result<Connection> {
    let codecs = installed_codecs(cx);
    Connection::with_session(
        ServerAddress::Local,
        first_codec(&codecs),
        codecs,
        site,
        role,
        sim_lib_server::IsolationPolicy::default(),
    )
}

pub(super) fn evaluate_connection(
    cx: &mut Cx,
    connection: &Connection,
    expr: Expr,
    role: Option<Symbol>,
    parent: &FrameEnvelope,
) -> Result<ServerFrame> {
    let request = EvalRequest {
        expr,
        mode: sim_kernel::EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: parent.required_capabilities.clone(),
        deadline: parent.deadline,
        consistency: parent.consistency,
        trace: parent.trace,
    };
    let mut frame = server_frame_from_request(cx, connection.default_codec(), request)?;
    frame.envelope.reply_codec_hint = parent.reply_codec_hint.clone();
    frame.envelope.trigger_source = parent.trigger_source.clone();
    frame.envelope.role = role.or_else(|| connection.role().cloned());
    frame.envelope.hop = parent.hop.saturating_add(1);
    connection.site().answer(cx, frame)
}

pub(super) fn reply_expr(cx: &mut Cx, frame: &ServerFrame) -> Result<Expr> {
    eval_reply_from_frame(cx, frame)?.value.object().as_expr(cx)
}

pub(super) fn is_stream_passthrough_frame(cx: &mut Cx, frame: &ServerFrame) -> Result<bool> {
    match frame.kind {
        FrameKind::Request | FrameKind::Response => Ok(false),
        _ => match stream_frame_to_expr(cx, frame) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        },
    }
}

pub(super) fn first_codec(codecs: &[Symbol]) -> Symbol {
    codecs
        .first()
        .cloned()
        .unwrap_or_else(|| Symbol::qualified("codec", "binary"))
}

pub(super) fn until_value(cx: &mut Cx) -> Result<Value> {
    cx.factory().opaque(Arc::new(TopologyUntilFn))
}

pub(super) fn pipeline_address(steps: &[Connection]) -> ServerAddress {
    ServerAddress::Pipeline {
        steps: steps.iter().map(|step| step.address().clone()).collect(),
    }
}

pub(super) fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

pub(super) use sim_value::access::field as map_field;

pub(super) fn bool_field(expr: &Expr, key: &str) -> bool {
    matches!(map_field(expr, key), Some(Expr::Bool(true)))
}

pub(super) fn expr_field(expr: &Expr, key: &str) -> Result<Expr> {
    map_field(expr, key)
        .cloned()
        .ok_or_else(|| Error::Eval(format!("missing {key} field")))
}

pub(super) fn number_field(expr: &Expr, key: &str) -> Result<f64> {
    match expr_field(expr, key)? {
        Expr::Number(number) => number
            .canonical
            .parse::<f64>()
            .map_err(|_| Error::Eval(format!("field {key} was not numeric"))),
        Expr::String(text) => text
            .parse::<f64>()
            .map_err(|_| Error::Eval(format!("field {key} was not numeric"))),
        other => Err(Error::Eval(format!(
            "field {key} was not numeric: {other:?}"
        ))),
    }
}

pub(super) fn text_label(value: &Value) -> String {
    if let Some(agent) = value.object().downcast_ref::<Agent>() {
        return agent.name.to_string();
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        return component.symbol.to_string();
    }
    if let Some(connection) = value.object().downcast_ref::<Connection>()
        && let Some(role) = connection.role()
    {
        return role.to_string();
    }
    "member".to_owned()
}

impl Callable for TopologyUntilFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let [state] = args.values() else {
            return Err(Error::Eval(
                "topology until predicate expects one state".to_owned(),
            ));
        };
        let done = bool_field(&state.object().as_expr(cx)?, "done");
        cx.factory().bool(done)
    }
}

impl Object for TopologyUntilFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<topology-until>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for TopologyUntilFn {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("topology", "Until"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
