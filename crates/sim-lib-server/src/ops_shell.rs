use std::sync::Arc;

use sim_kernel::{Cx, Error, Expr, RawArgs, Result, Symbol, Value, eval_fabric_capability};

use crate::helpers::{
    connect_target_value, ensure_installed_codec, literal_expr, normalize_codec_expr,
    parse_server_options, symbol_from_value, symbol_of,
};
use crate::isolation::IsolationPolicy;
use crate::trigger::register_trigger;
use crate::{Server, ServerAddress, TriggerHandle, repl};

#[cfg(feature = "wasm")]
use crate::{
    helpers::{string_like_from_value, wasm_module_bytes_from_value},
    register_wasm_region,
};
#[cfg(feature = "wasm")]
use sim_kernel::CapabilityName;

#[cfg(feature = "wasm")]
const SANDBOX_WASM_CAPABILITY: &str = "sandbox-wasm";

pub(crate) fn server_repl(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    let exprs = args.into_exprs();

    let mut target: Option<Value> = None;
    let mut codec: Option<Symbol> = None;
    let mut prompt = "sim> ".to_owned();
    let mut driver = repl::DriverSpec::Line;
    let mut input = None;
    let mut output = repl::ReplOutput::Driver;
    let mut isolation = IsolationPolicy::default();

    parse_server_options(cx, &exprs, "server/repl", |cx, key, expr| match key {
        "address" => {
            ServerAddress::from_expr(expr)?;
            target = Some(cx.factory().expr(expr.clone())?);
            Ok(())
        }
        "connection" => {
            target = Some(cx.eval_expr(expr.clone())?);
            Ok(())
        }
        "codec" => {
            let value = cx.eval_expr(expr.clone())?;
            let parsed = symbol_from_value(cx, value, "server/repl :codec expects a symbol")?;
            ensure_installed_codec(cx, &parsed)?;
            codec = Some(parsed);
            Ok(())
        }
        "prompt" => {
            let value = cx.eval_expr(expr.clone())?;
            prompt = match value.object().as_expr(cx)? {
                Expr::String(text) => text,
                _ => {
                    return Err(Error::Eval(
                        "server/repl :prompt expects a string".to_owned(),
                    ));
                }
            };
            Ok(())
        }
        "driver" => {
            driver = repl::DriverSpec::from_expr(expr)?;
            Ok(())
        }
        "input" => {
            let value = cx.eval_expr(expr.clone())?;
            input = match value.object().as_expr(cx)? {
                Expr::String(text) => Some(text),
                _ => {
                    return Err(Error::Eval(
                        "server/repl :input expects a string".to_owned(),
                    ));
                }
            };
            Ok(())
        }
        "output" => {
            let value = cx.eval_expr(expr.clone())?;
            let symbol = symbol_from_value(cx, value, "server/repl :output expects a symbol")?;
            output = match symbol.name.as_ref() {
                "string" => repl::ReplOutput::String,
                other => {
                    return Err(Error::Eval(format!(
                        "server/repl: unsupported :output {other}"
                    )));
                }
            };
            Ok(())
        }
        "isolate" => {
            isolation = IsolationPolicy::from_expr(expr)?;
            Ok(())
        }
        other => Err(Error::Eval(format!("server/repl: unknown option :{other}"))),
    })?;

    let target = match target {
        Some(target) => target,
        None => cx.factory().expr(Expr::Symbol(Symbol::new("local")))?,
    };
    let connection = connect_target_value(cx, target, None, &[], None, false, isolation)?;
    let repl_codec = codec.unwrap_or_else(|| connection.default_codec().clone());
    repl::run_repl(
        cx,
        repl::ReplOptions {
            connection,
            codec: repl_codec,
            prompt,
            driver,
            input,
            output,
        },
    )
}

#[cfg(feature = "wasm")]
pub(crate) fn server_wasm_region(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    cx.require(&CapabilityName::new(SANDBOX_WASM_CAPABILITY))?;
    let exprs = args.into_exprs();
    let mut region = None;
    let mut module = None;

    parse_server_options(
        cx,
        &exprs,
        "server/wasm-region",
        |cx, key, expr| match key {
            "name" => {
                let value = cx.eval_expr(expr.clone())?;
                region = Some(string_like_from_value(
                    cx,
                    value,
                    "server/wasm-region :name expects a string or symbol",
                )?);
                Ok(())
            }
            "module" => {
                let value = cx.eval_expr(expr.clone())?;
                module = Some(wasm_module_bytes_from_value(
                    cx,
                    value,
                    "server/wasm-region :module expects bytes or a file path string",
                )?);
                Ok(())
            }
            other => Err(Error::Eval(format!(
                "server/wasm-region: unknown option :{other}"
            ))),
        },
    )?;

    let region =
        region.ok_or_else(|| Error::Eval("server/wasm-region requires :name".to_owned()))?;
    let module =
        module.ok_or_else(|| Error::Eval("server/wasm-region requires :module".to_owned()))?;
    register_wasm_region(&region, &module)?;
    cx.factory().nil()
}

#[cfg(not(feature = "wasm"))]
pub(crate) fn server_wasm_region(_cx: &mut Cx, _args: RawArgs) -> Result<Value> {
    Err(Error::Eval("server wasm feature disabled".to_owned()))
}

pub(crate) fn server_trigger(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(server_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/trigger expects a server and keyword options".to_owned(),
        ));
    };
    let server_value = cx.eval_expr(server_expr.clone())?;
    let server = server_value
        .object()
        .downcast_ref::<Server>()
        .cloned()
        .ok_or(Error::TypeMismatch {
            expected: "server",
            found: "non-server",
        })?;

    let mut source = None;
    let mut decode = None;
    let mut role = None;
    let mut codec = Some(server.default_codec().clone());

    parse_server_options(
        cx,
        &exprs[1..],
        "server/trigger",
        |cx, key, expr| match key {
            "source" => {
                source = Some(expr.clone());
                Ok(())
            }
            "decode" => {
                decode = Some(expr.clone());
                Ok(())
            }
            "role" => {
                role = Some(symbol_of(
                    literal_expr(expr),
                    "server/trigger :role expects a symbol",
                )?);
                Ok(())
            }
            "codec" => {
                let parsed =
                    symbol_of(literal_expr(expr), "server/trigger :codec expects a symbol")?;
                codec = Some(normalize_codec_expr(cx, &parsed).unwrap_or(parsed));
                Ok(())
            }
            other => Err(Error::Eval(format!(
                "server/trigger: unknown option :{other}"
            ))),
        },
    )?;

    let trigger = register_trigger(
        cx,
        Arc::new(server),
        source.ok_or_else(|| Error::Eval("server/trigger requires :source".to_owned()))?,
        decode.ok_or_else(|| Error::Eval("server/trigger requires :decode".to_owned()))?,
        role,
        codec.ok_or_else(|| Error::Eval("server/trigger requires :codec".to_owned()))?,
    )?;
    cx.factory().opaque(trigger)
}

pub(crate) fn server_trigger_poll(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let exprs = args.into_exprs();
    let Some(trigger_expr) = exprs.first() else {
        return Err(Error::Eval(
            "server/trigger-poll expects one trigger value".to_owned(),
        ));
    };
    let trigger_value = cx.eval_expr(trigger_expr.clone())?;
    let trigger = trigger_value
        .object()
        .downcast_ref::<TriggerHandle>()
        .ok_or(Error::TypeMismatch {
            expected: "trigger",
            found: "non-trigger",
        })?;
    let delivered = trigger.poll(cx)?;
    cx.factory().string(delivered.to_string())
}
