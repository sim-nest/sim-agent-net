use std::sync::Arc;

use sim_kernel::{Cx, Error, Expr, RawArgs, Result, Symbol, Value};

use crate::helpers::{
    bool_from_value, connect_target_value, coroutine_target_from_value, ensure_installed_codec,
    evaluated_connection, parse_server_options, pipeline_connection_from_steps,
    pipeline_steps_from_expr, server_from_value, symbol_from_value, symbol_list_from_value,
    usize_from_value,
};
use crate::{LoopEvalSite, Server, ServerStatus, isolation::IsolationPolicy};

pub(crate) fn server_connect(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(target_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/connect expects an address expression or server value".to_owned(),
        ));
    };

    let target = cx.eval_expr(target_expr.clone())?;
    let mut codec = None;
    let mut preferred = Vec::new();
    let mut role = None;
    let mut loopback = false;
    let mut isolation = IsolationPolicy::default();

    parse_server_options(
        cx,
        &exprs[1..],
        "server/connect",
        |cx, key, expr| match key {
            "codec" => {
                let value = cx.eval_expr(expr.clone())?;
                let parsed =
                    symbol_from_value(cx, value, "server/connect :codec expects a symbol")?;
                ensure_installed_codec(cx, &parsed)?;
                codec = Some(parsed);
                Ok(())
            }
            "preferred" => {
                let value = cx.eval_expr(expr.clone())?;
                preferred = symbol_list_from_value(cx, value)?;
                for preferred_codec in &preferred {
                    ensure_installed_codec(cx, preferred_codec)?;
                }
                Ok(())
            }
            "role" => {
                let value = cx.eval_expr(expr.clone())?;
                role = Some(symbol_from_value(
                    cx,
                    value,
                    "server/connect :role expects a symbol",
                )?);
                Ok(())
            }
            "loopback" => {
                let value = cx.eval_expr(expr.clone())?;
                loopback = bool_from_value(cx, value, "server/connect :loopback expects a bool")?;
                Ok(())
            }
            "isolate" => {
                isolation = IsolationPolicy::from_expr(expr)?;
                Ok(())
            }
            other => Err(Error::Eval(format!(
                "server/connect: unknown option :{other}"
            ))),
        },
    )?;

    let connection =
        connect_target_value(cx, target, codec, &preferred, role, loopback, isolation)?;
    cx.factory().opaque(Arc::new(connection))
}

pub(crate) fn server_pipeline(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let mut codec = None;
    let mut step_expr = None;

    parse_server_options(cx, &exprs, "server/pipeline", |cx, key, expr| match key {
        "steps" => {
            step_expr = Some(expr.clone());
            Ok(())
        }
        "codec" => {
            let value = cx.eval_expr(expr.clone())?;
            let parsed = symbol_from_value(cx, value, "server/pipeline :codec expects a symbol")?;
            ensure_installed_codec(cx, &parsed)?;
            codec = Some(parsed);
            Ok(())
        }
        other => Err(Error::Eval(format!(
            "server/pipeline: unknown option :{other}"
        ))),
    })?;

    let steps = pipeline_steps_from_expr(
        cx,
        step_expr.ok_or_else(|| Error::Eval("server/pipeline requires :steps".to_owned()))?,
    )?;
    let connection = pipeline_connection_from_steps(cx, steps, codec)?;
    cx.factory().opaque(Arc::new(connection))
}

pub(crate) fn server_loop(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let mut codec = None;
    let mut step_expr = None;
    let mut max_iterations = None;
    let mut until = None;

    parse_server_options(cx, &exprs, "server/loop", |cx, key, expr| match key {
        "steps" => {
            step_expr = Some(expr.clone());
            Ok(())
        }
        "codec" => {
            let value = cx.eval_expr(expr.clone())?;
            let parsed = symbol_from_value(cx, value, "server/loop :codec expects a symbol")?;
            ensure_installed_codec(cx, &parsed)?;
            codec = Some(parsed);
            Ok(())
        }
        "max-iterations" => {
            let value = cx.eval_expr(expr.clone())?;
            max_iterations = Some(usize_from_value(
                cx,
                value,
                "server/loop :max-iterations expects a positive integer",
            )?);
            Ok(())
        }
        "until" => {
            let value = cx.eval_expr(expr.clone())?;
            if value.object().as_callable().is_none() {
                return Err(Error::TypeMismatch {
                    expected: "callable",
                    found: "non-callable",
                });
            }
            until = Some(value);
            Ok(())
        }
        other => Err(Error::Eval(format!("server/loop: unknown option :{other}"))),
    })?;

    let steps = pipeline_steps_from_expr(
        cx,
        step_expr.ok_or_else(|| Error::Eval("server/loop requires :steps".to_owned()))?,
    )?;
    let max_iterations = max_iterations
        .ok_or_else(|| Error::Eval("server/loop requires :max-iterations".to_owned()))?;
    let until = until.ok_or_else(|| Error::Eval("server/loop requires :until".to_owned()))?;
    let connection =
        crate::helpers::loop_connection_from_steps(cx, steps, codec, max_iterations, until)?;
    cx.factory().opaque(Arc::new(connection))
}

pub(crate) fn server_start_loop(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(loop_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/start-loop expects one loop connection".to_owned(),
        ));
    };
    let connection = evaluated_connection(cx, loop_expr)?;
    if connection
        .site()
        .as_any()
        .downcast_ref::<LoopEvalSite>()
        .is_none()
    {
        return Err(Error::TypeMismatch {
            expected: "loop connection",
            found: "non-loop connection",
        });
    }
    cx.factory().opaque(Arc::new(connection))
}

pub(crate) fn server_resume_exprs(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(target_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/resume expects a server or coroutine value".to_owned(),
        ));
    };
    let target = cx.eval_expr(target_expr.clone())?;
    if let Some(server) = server_from_value(&target) {
        if exprs.len() == 1 {
            server.set_status(ServerStatus::Running);
            return Ok(target);
        }
        if let Ok(coroutine) = coroutine_target_from_value(&target) {
            let input = cx.factory().expr(exprs[1].clone())?;
            return coroutine.resume(cx, input);
        }
        return Err(Error::Eval(
            "server/resume only accepts an input expression for coroutine-backed servers"
                .to_owned(),
        ));
    }
    let coroutine = coroutine_target_from_value(&target)?;
    let Some(input_expr) = exprs.get(1) else {
        return Err(Error::Eval(
            "server/resume expects an input expression for coroutine targets".to_owned(),
        ));
    };
    let input = cx.factory().expr(input_expr.clone())?;
    coroutine.resume(cx, input)
}

pub(crate) fn server_yield(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    if exprs.len() != 2 {
        return Err(Error::Eval(
            "server/yield expects a coroutine and a value".to_owned(),
        ));
    }
    let coroutine_value = cx.eval_expr(exprs[0].clone())?;
    let yielded = cx.eval_expr(exprs[1].clone())?;
    let coroutine = coroutine_target_from_value(&coroutine_value)?;
    coroutine.yield_value(yielded)
}

pub(crate) fn server_coroutine_status(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(target_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/coroutine-status expects a coroutine or server value".to_owned(),
        ));
    };
    let target = cx.eval_expr(target_expr.clone())?;
    let coroutine = coroutine_target_from_value(&target)?;
    cx.factory().symbol(coroutine.status().as_symbol())
}

pub(crate) fn server_cancel_coroutine(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(target_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/cancel-coroutine expects a coroutine or server value".to_owned(),
        ));
    };
    let target = cx.eval_expr(target_expr.clone())?;
    let coroutine = coroutine_target_from_value(&target)?;
    coroutine.cancel()?;
    cx.factory().nil()
}

pub(crate) fn server_lisp(server: &Server) -> Expr {
    let mut call_args = Vec::new();
    for (key, value) in server.spec() {
        call_args.push(Expr::Symbol(Symbol::new(format!(":{}", key.name))));
        call_args.push(value.clone());
    }
    Expr::Call {
        operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
        args: call_args,
    }
}
