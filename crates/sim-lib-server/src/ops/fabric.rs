use std::sync::Arc;

use sim_kernel::{Cx, Error, RawArgs, Result, Value, eval_fabric_capability};

use crate::helpers::{
    capability_names_from_value, coerce_result_shape, evaluated_connection, keyword,
    parse_consistency_value, parse_duration_value, parse_message_options, resolve_on_target,
    symbol_from_value,
};
use crate::stream_support::{BufferedStreamSink, StreamHandle, evaluated_stream_handle};
use crate::{FrameKind, ServerFrame, StreamSink};

pub(crate) fn server_send(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    let exprs = args.into_exprs();
    if exprs.len() < 2 {
        return Err(Error::Eval(
            "server/send expects a connection and an expression".to_owned(),
        ));
    }
    let conn = evaluated_connection(cx, &exprs[0])?;
    let mut codec = conn.default_codec().clone();
    let mut deadline = None;
    let mut required_capabilities = Vec::new();
    let mut reply_codec_hint = None;
    let mut consistency = conn.default_consistency();
    parse_message_options(
        cx,
        &exprs[2..],
        "server/send",
        &mut codec,
        &mut deadline,
        &mut required_capabilities,
        Some(&mut reply_codec_hint),
        Some(&mut consistency),
    )?;
    let msg_id = conn.send_with_consistency(
        cx,
        exprs[1].clone(),
        codec,
        deadline,
        required_capabilities,
        reply_codec_hint,
        consistency,
    )?;
    cx.factory().string(msg_id.to_string())
}

pub(crate) fn server_request(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    let exprs = args.into_exprs();
    if exprs.len() < 2 {
        return Err(Error::Eval(
            "server/request expects a connection and an expression".to_owned(),
        ));
    }
    let conn = evaluated_connection(cx, &exprs[0])?;

    let mut timeout = None;
    let mut required_capabilities = Vec::new();
    let mut consistency = conn.default_consistency();
    if !exprs[2..].len().is_multiple_of(2) {
        return Err(Error::Eval(
            "server/request options must be key/value pairs".to_owned(),
        ));
    }
    for pair in exprs[2..].chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "timeout" => {
                let value = cx.eval_expr(pair[1].clone())?;
                timeout = Some(parse_duration_value(cx, value)?);
            }
            "codec" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let requested =
                    symbol_from_value(cx, value, "server/request :codec expects a symbol")?;
                if requested != *conn.default_codec() {
                    return Err(Error::Eval(
                        "server/request currently uses the connection default codec".to_owned(),
                    ));
                }
            }
            "requires" => {
                let value = cx.eval_expr(pair[1].clone())?;
                required_capabilities = capability_names_from_value(cx, value)?;
            }
            "consistency" => {
                let value = cx.eval_expr(pair[1].clone())?;
                consistency = parse_consistency_value(cx, value)?;
            }
            other => {
                return Err(Error::Eval(format!(
                    "server/request: unknown option :{other}"
                )));
            }
        }
    }

    conn.request_with_consistency(
        cx,
        exprs[1].clone(),
        timeout,
        required_capabilities,
        consistency,
    )
}

pub(crate) fn server_notify(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    let exprs = args.into_exprs();
    if exprs.len() < 2 {
        return Err(Error::Eval(
            "server/notify expects a connection and an expression".to_owned(),
        ));
    }
    let conn = evaluated_connection(cx, &exprs[0])?;
    let mut codec = conn.default_codec().clone();
    let mut deadline = None;
    let mut required_capabilities = Vec::new();
    parse_message_options(
        cx,
        &exprs[2..],
        "server/notify",
        &mut codec,
        &mut deadline,
        &mut required_capabilities,
        None,
        None,
    )?;
    conn.notify(cx, exprs[1].clone(), codec, deadline, required_capabilities)?;
    cx.factory().nil()
}

pub(crate) fn server_receive(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(conn_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/receive expects a connection".to_owned(),
        ));
    };
    let conn = evaluated_connection(cx, conn_expr)?;
    let mut timeout = None;
    if !exprs[1..].len().is_multiple_of(2) {
        return Err(Error::Eval(
            "server/receive options must be key/value pairs".to_owned(),
        ));
    }
    for pair in exprs[1..].chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "timeout" => {
                let value = cx.eval_expr(pair[1].clone())?;
                timeout = Some(parse_duration_value(cx, value)?);
            }
            other => {
                return Err(Error::Eval(format!(
                    "server/receive: unknown option :{other}"
                )));
            }
        }
    }
    match conn.receive(timeout)? {
        Some(frame) => cx.factory().opaque(Arc::new(frame)),
        None => cx.factory().nil(),
    }
}

pub(crate) fn server_stream(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    let exprs = args.into_exprs();
    if exprs.len() < 2 {
        return Err(Error::Eval(
            "server/stream expects a connection and an expression".to_owned(),
        ));
    }
    let conn = evaluated_connection(cx, &exprs[0])?;

    let mut codec = conn.default_codec().clone();
    let mut deadline = None;
    let mut required_capabilities = Vec::new();
    parse_message_options(
        cx,
        &exprs[2..],
        "server/stream",
        &mut codec,
        &mut deadline,
        &mut required_capabilities,
        None,
        None,
    )?;

    let mut frame = ServerFrame::from_expr(
        cx,
        codec,
        FrameKind::Request,
        &exprs[1],
        if conn.address().is_remote_like() || conn.site().address().is_remote_like() {
            sim_kernel::Consistency::RemoteOnly
        } else {
            sim_kernel::Consistency::LocalFirst
        },
        required_capabilities,
        false,
    )?;
    frame.envelope.deadline = deadline;
    frame.envelope.role = conn.role().cloned();

    let handle = Arc::new(StreamHandle::default());
    let mut sink = BufferedStreamSink::new(handle.clone());
    match conn.site().stream(cx, frame, &mut sink) {
        Ok(()) => sink.end(cx)?,
        Err(err) => handle.finish_with_error(format!("{err}")),
    }
    cx.factory().opaque(handle)
}

pub(crate) fn server_stream_next(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(handle_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/stream-next expects a stream handle".to_owned(),
        ));
    };
    let handle = evaluated_stream_handle(cx, handle_expr)?;

    if !exprs[1..].len().is_multiple_of(2) {
        return Err(Error::Eval(
            "server/stream-next options must be key/value pairs".to_owned(),
        ));
    }
    for pair in exprs[1..].chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "timeout" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let _ = parse_duration_value(cx, value)?;
            }
            other => {
                return Err(Error::Eval(format!(
                    "server/stream-next: unknown option :{other}"
                )));
            }
        }
    }

    handle.next(cx)
}

pub(crate) fn server_realize(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/realize expects an expression argument".to_owned(),
        ));
    };

    let mut on = None;
    let mut result_shape = None;
    let mut required_capabilities = Vec::new();
    let mut deadline = None;
    let mut consistency = sim_kernel::Consistency::LocalFirst;
    let mut mode = sim_kernel::EvalMode::Eval;
    let mut answer_limit = None;
    let mut stream_buffer = None;
    let mut stream = false;
    let mut trace = false;

    if !exprs[1..].len().is_multiple_of(2) {
        return Err(Error::Eval(
            "server/realize options must be key/value pairs".to_owned(),
        ));
    }
    for pair in exprs[1..].chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "on" => on = Some(cx.eval_expr(pair[1].clone())?),
            "result" => {
                let value = cx.eval_expr(pair[1].clone())?;
                result_shape = coerce_result_shape(cx, value)?;
            }
            "requires" => {
                let value = cx.eval_expr(pair[1].clone())?;
                required_capabilities = capability_names_from_value(cx, value)?;
            }
            "deadline" | "timeout" => {
                let value = cx.eval_expr(pair[1].clone())?;
                deadline = Some(parse_duration_value(cx, value)?)
            }
            "consistency" => {
                let value = cx.eval_expr(pair[1].clone())?;
                consistency = parse_consistency_value(cx, value)?
            }
            "mode" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let expr = value.object().as_expr(cx)?;
                mode = match expr {
                    sim_kernel::Expr::Symbol(symbol) if symbol.name.as_ref() == "logic" => {
                        sim_kernel::EvalMode::Logic
                    }
                    sim_kernel::Expr::Symbol(_) | sim_kernel::Expr::String(_) => {
                        sim_kernel::EvalMode::Eval
                    }
                    _ => {
                        return Err(Error::TypeMismatch {
                            expected: "mode symbol",
                            found: "non-mode",
                        });
                    }
                };
            }
            "answer-limit" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let expr = value.object().as_expr(cx)?;
                answer_limit = match expr {
                    sim_kernel::Expr::Number(number) => {
                        Some(number.canonical.parse::<usize>().map_err(|_| {
                            Error::Eval("server/realize :answer-limit expects usize".to_owned())
                        })?)
                    }
                    sim_kernel::Expr::String(text) => {
                        Some(text.parse::<usize>().map_err(|_| {
                            Error::Eval("server/realize :answer-limit expects usize".to_owned())
                        })?)
                    }
                    _ => {
                        return Err(Error::TypeMismatch {
                            expected: "usize",
                            found: "non-usize",
                        });
                    }
                };
            }
            "buffer" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let expr = value.object().as_expr(cx)?;
                stream_buffer = match expr {
                    sim_kernel::Expr::Number(number) => {
                        Some(number.canonical.parse::<usize>().map_err(|_| {
                            Error::Eval("server/realize :buffer expects usize".to_owned())
                        })?)
                    }
                    sim_kernel::Expr::String(text) => {
                        Some(text.parse::<usize>().map_err(|_| {
                            Error::Eval("server/realize :buffer expects usize".to_owned())
                        })?)
                    }
                    _ => {
                        return Err(Error::TypeMismatch {
                            expected: "usize",
                            found: "non-usize",
                        });
                    }
                };
            }
            "stream" => stream = cx.eval_expr(pair[1].clone())?.object().truth(cx)?,
            "trace" => trace = cx.eval_expr(pair[1].clone())?.object().truth(cx)?,
            other => {
                return Err(Error::Eval(format!(
                    "server/realize: unknown option :{other}"
                )));
            }
        }
    }

    let connection = resolve_on_target(
        cx,
        on.ok_or_else(|| Error::Eval("server/realize requires :on".to_owned()))?,
    )?;
    let reply = sim_kernel::realize_final(
        cx,
        &connection,
        sim_kernel::EvalRequest {
            expr: expr.clone(),
            result_shape,
            required_capabilities,
            deadline,
            consistency,
            mode,
            answer_limit,
            stream_buffer,
            stream,
            trace,
        },
    )?;
    Ok(reply.value)
}
