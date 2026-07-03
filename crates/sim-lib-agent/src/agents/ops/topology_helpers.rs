use super::topology_runtime::{
    DebateSession, MeshSession, RingSession, evaluate_connection, map_field, number_expr,
    number_field, reply_expr, text_label,
};
use crate::{expr_to_value, memory::flatten_expr_text};
use sim_kernel::{Cx, Error, EvalReply, Expr, Result, Symbol, Value};
use sim_lib_server::{FrameEnvelope, ServerAddress, ServerFrame, server_frame_from_reply};
use std::sync::{Arc, Mutex, OnceLock};

pub(super) fn reply_state(cx: &mut Cx, frame: &ServerFrame, state: Expr) -> Result<ServerFrame> {
    reply_expr_value(cx, frame, state)
}

pub(super) fn reply_expr_value(
    cx: &mut Cx,
    frame: &ServerFrame,
    expr: Expr,
) -> Result<ServerFrame> {
    let value = expr_to_value(cx, &expr)?;
    let diagnostics = cx.take_diagnostics();
    server_frame_from_reply(
        cx,
        &frame.codec,
        EvalReply {
            value,
            diagnostics,
            trace: None,
        },
        frame.envelope.consistency,
    )
}

pub(super) fn ring_state(session: &Arc<Mutex<RingSession>>) -> Result<Expr> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("ring session"))?;
    Ok(Expr::Map(vec![
        (Expr::Symbol(Symbol::new("done")), Expr::Bool(session.done)),
        (Expr::Symbol(Symbol::new("result")), session.current.clone()),
        (
            Expr::Symbol(Symbol::new("transcript")),
            Expr::List(session.transcript.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("turns-used")),
            number_expr(session.turns_used),
        ),
    ]))
}

pub(super) fn mesh_state(session: &Arc<Mutex<MeshSession>>) -> Result<Expr> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("mesh session"))?;
    Ok(Expr::Map(vec![
        (Expr::Symbol(Symbol::new("done")), Expr::Bool(session.done)),
        (
            Expr::Symbol(Symbol::new("candidate")),
            session.candidate.clone(),
        ),
        (
            Expr::Symbol(Symbol::new("score")),
            number_expr(session.best_score.unwrap_or(0.0)),
        ),
        (
            Expr::Symbol(Symbol::new("transcript")),
            Expr::List(session.transcript.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("rounds-used")),
            number_expr(session.rounds_used),
        ),
    ]))
}

pub(super) fn debate_state(session: &Arc<Mutex<DebateSession>>) -> Result<Expr> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("debate session"))?;
    Ok(Expr::Map(vec![
        (Expr::Symbol(Symbol::new("done")), Expr::Bool(session.done)),
        (Expr::Symbol(Symbol::new("task")), session.task.clone()),
        (
            Expr::Symbol(Symbol::new("transcript")),
            Expr::List(session.transcript.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("turns-used")),
            number_expr(session.turns_used),
        ),
    ]))
}

pub(super) fn judge_expr(
    cx: &mut Cx,
    judge: &Value,
    expr: Expr,
    parent: &FrameEnvelope,
) -> Result<Expr> {
    let connection = super::shared::agent_connection_for_value(judge.clone())?;
    let reply = evaluate_connection(cx, &connection, expr, Some(Symbol::new("judge")), parent)?;
    reply_expr(cx, &reply)
}

pub(super) fn judge_score(
    cx: &mut Cx,
    judge: &Value,
    candidate: Expr,
    parent: &FrameEnvelope,
) -> Result<f64> {
    let verdict = judge_expr(cx, judge, candidate, parent)?;
    number_field(&verdict, "score")
}

pub(super) fn side_case(transcript: &[Expr], side: &str) -> Expr {
    Expr::String(
        transcript
            .iter()
            .filter_map(|entry| {
                let matches_side = matches!(
                    map_field(entry, "side"),
                    Some(Expr::String(actual)) if actual == side
                );
                if !matches_side {
                    return None;
                }
                map_field(entry, "value").map(flatten_expr_text)
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub(super) fn select_market_winner(
    cx: &mut Cx,
    workers: &[Value],
    router: &Value,
    task: &Expr,
    parent: &FrameEnvelope,
) -> Result<(Value, f64)> {
    let router_conn = super::shared::agent_connection_for_value(router.clone())?;
    let routed = evaluate_connection(
        cx,
        &router_conn,
        task.clone(),
        Some(Symbol::new("router")),
        parent,
    )?;
    let decision = reply_expr(cx, &routed)?;
    if let Some(Expr::String(label)) = map_field(&decision, "target")
        && let Some(worker) = workers.iter().find(|worker| text_label(worker) == *label)
    {
        return Ok((
            worker.clone(),
            number_field(&decision, "bid").unwrap_or(0.0),
        ));
    }
    if let Some(Expr::Symbol(label)) = map_field(&decision, "target")
        && let Some(worker) = workers
            .iter()
            .find(|worker| text_label(worker) == label.to_string())
    {
        return Ok((
            worker.clone(),
            number_field(&decision, "bid").unwrap_or(0.0),
        ));
    }
    let mut best = None::<(Value, f64)>;
    for worker in workers {
        let connection = super::shared::agent_connection_for_value(worker.clone())?;
        let reply = evaluate_connection(
            cx,
            &connection,
            Expr::List(vec![Expr::Symbol(Symbol::new("bid")), task.clone()]),
            Some(Symbol::new("worker")),
            parent,
        )?;
        let bid_expr = reply_expr(cx, &reply)?;
        let bid = match bid_expr {
            Expr::Number(number) => number.canonical.parse::<f64>().unwrap_or(f64::INFINITY),
            Expr::Map(_) => number_field(&bid_expr, "bid").unwrap_or(f64::INFINITY),
            _ => f64::INFINITY,
        };
        if best.as_ref().is_none_or(|(_, current)| bid < *current) {
            best = Some((worker.clone(), bid));
        }
    }
    best.ok_or_else(|| Error::Eval("market auction found no bidders".to_owned()))
}

pub(super) fn local_address() -> &'static ServerAddress {
    static LOCAL: OnceLock<ServerAddress> = OnceLock::new();
    LOCAL.get_or_init(|| ServerAddress::Local)
}
