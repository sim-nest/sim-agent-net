use crate::agents::site_from_value;
use crate::components::model::{AgentComponent, RouterBackend};
use crate::memory::flatten_expr_text;
use crate::util::expr_to_value;
use sim_kernel::{Cx, Error, EvalRequest, Expr, ReadPolicy, Result, Symbol};
use sim_lib_server::{
    FrameKind, ServerFrame, eval_reply_from_frame, eval_request_from_frame,
    server_frame_from_request, stream_frame_to_expr,
};
use std::sync::{Arc, Mutex};

pub(in crate::components) fn answer_router(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RouterBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        stream_frame_to_expr(cx, &frame).map_err(|err| {
            Error::Eval(format!(
                "{} only routes request or stream frames: {err}",
                component.symbol
            ))
        })?;
        return Ok(frame);
    }
    let consistency = frame.envelope.consistency;
    let decision = route_decision_expr(cx, backend, &frame)?;
    let value = expr_to_value(cx, &decision)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
        .map_err(|err| Error::Eval(format!("{} failed to route: {err}", component.symbol)))
}

fn route_decision_expr(cx: &mut Cx, backend: &RouterBackend, frame: &ServerFrame) -> Result<Expr> {
    match backend {
        RouterBackend::RoundRobin { targets, cursor } => Ok(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("target")),
                Expr::Symbol(select_round_robin_target(targets, cursor)?),
            ),
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(frame.kind.as_symbol()),
            ),
        ])),
        RouterBackend::Bid {
            targets,
            metric,
            auction_window,
        } => bid_route_expr(cx, frame, targets, metric, *auction_window),
        RouterBackend::Sticky {
            targets,
            sticky_key,
        } => Ok(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("target")),
                Expr::Symbol(select_sticky_target(cx, targets, sticky_key, frame)?),
            ),
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(frame.kind.as_symbol()),
            ),
        ])),
    }
}

fn bid_route_expr(
    cx: &mut Cx,
    frame: &ServerFrame,
    targets: &[Symbol],
    metric: &Symbol,
    auction_window: std::time::Duration,
) -> Result<Expr> {
    let task = match frame.kind {
        FrameKind::Request => eval_request_from_frame(cx, frame)?.expr,
        _ => frame.decode_expr(cx, ReadPolicy::default())?,
    };
    let mut winner = None::<(Symbol, f64)>;
    for target in targets {
        let Ok(value) = cx.resolve_value(target) else {
            continue;
        };
        let Ok(site) = site_from_value(&value) else {
            continue;
        };
        let bid_request = EvalRequest {
            expr: Expr::List(vec![Expr::Symbol(Symbol::new("bid")), task.clone()]),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: frame.envelope.required_capabilities.clone(),
            deadline: Some(auction_window),
            consistency: frame.envelope.consistency,
            trace: frame.envelope.trace,
        };
        let request_codec = site
            .codecs()
            .first()
            .cloned()
            .unwrap_or_else(|| frame.codec.clone());
        let request_frame = server_frame_from_request(cx, &request_codec, bid_request)?;
        let Ok(reply_frame) = site.answer_with_timeout(cx, request_frame, Some(auction_window))
        else {
            continue;
        };
        let Ok(reply) = eval_reply_from_frame(cx, &reply_frame) else {
            continue;
        };
        let Ok(bid) = bid_value(cx, &reply.value) else {
            continue;
        };
        if is_better_bid(metric, bid, winner.as_ref().map(|(_, current)| *current)) {
            winner = Some((target.clone(), bid));
        }
    }

    let (target, bid) = winner.unwrap_or((Symbol::new("local"), default_bid(metric)));
    Ok(Expr::Map(vec![
        (Expr::Symbol(Symbol::new("target")), Expr::Symbol(target)),
        (
            Expr::Symbol(Symbol::new("kind")),
            Expr::Symbol(frame.kind.as_symbol()),
        ),
        (
            Expr::Symbol(Symbol::new("metric")),
            Expr::Symbol(metric.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("bid")),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: bid.to_string(),
            }),
        ),
    ]))
}

fn bid_value(cx: &mut Cx, value: &sim_kernel::Value) -> Result<f64> {
    match value.object().as_expr(cx)? {
        Expr::Number(number) => number
            .canonical
            .parse::<f64>()
            .map_err(|_| Error::Eval("router bid reply was not numeric".to_owned())),
        Expr::Map(entries) => entries
            .into_iter()
            .find_map(|(key, value)| match key {
                Expr::Symbol(symbol)
                    if matches!(
                        symbol.name.as_ref(),
                        "bid" | "cost" | "score" | "estimated-cost" | "value"
                    ) =>
                {
                    Some(value)
                }
                _ => None,
            })
            .ok_or_else(|| Error::Eval("router bid reply missing bid field".to_owned()))
            .and_then(|expr| match expr {
                Expr::Number(number) => number
                    .canonical
                    .parse::<f64>()
                    .map_err(|_| Error::Eval("router bid reply was not numeric".to_owned())),
                _ => Err(Error::Eval("router bid reply was not numeric".to_owned())),
            }),
        _ => Err(Error::Eval("router bid reply was not numeric".to_owned())),
    }
}

fn is_better_bid(metric: &Symbol, bid: f64, current: Option<f64>) -> bool {
    let Some(current) = current else {
        return true;
    };
    if metric.name.as_ref().contains("cost") || metric.name.as_ref().contains("latency") {
        bid < current
    } else {
        bid > current
    }
}

fn default_bid(metric: &Symbol) -> f64 {
    if metric.name.as_ref().contains("cost") {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    }
}

fn select_round_robin_target(targets: &[Symbol], cursor: &Arc<Mutex<usize>>) -> Result<Symbol> {
    if targets.is_empty() {
        return Ok(Symbol::new("local"));
    }
    let mut cursor = cursor
        .lock()
        .map_err(|_| Error::PoisonedLock("router round-robin"))?;
    let target = targets[*cursor % targets.len()].clone();
    *cursor = cursor.saturating_add(1);
    Ok(target)
}

fn select_sticky_target(
    cx: &mut Cx,
    targets: &[Symbol],
    sticky_key: &Symbol,
    frame: &ServerFrame,
) -> Result<Symbol> {
    if targets.is_empty() {
        return Ok(Symbol::new("local"));
    }
    let expr = frame.decode_expr(cx, ReadPolicy::default())?;
    let sticky = format!("{}:{}", sticky_key, flatten_expr_text(&expr));
    let hash = sticky.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as usize)
    });
    Ok(targets[hash % targets.len()].clone())
}
