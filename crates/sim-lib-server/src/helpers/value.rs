#[cfg(feature = "wasm")]
use std::fs;

use crate::{Connection, Server};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, HandleSeed, Result, Value};
use sim_value::capability_names_from_expr;

pub(crate) fn symbol_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<sim_kernel::Symbol> {
    match value.object().as_expr(cx)? {
        Expr::Symbol(symbol) => Ok(symbol),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

#[cfg(feature = "wasm")]
pub(crate) fn string_like_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<String> {
    match value.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

#[cfg(feature = "wasm")]
pub(crate) fn wasm_module_bytes_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<Vec<u8>> {
    match value.object().as_expr(cx)? {
        Expr::Bytes(bytes) => Ok(bytes),
        Expr::String(path) => fs::read(&path)
            .map_err(|err| Error::HostError(format!("failed to read wasm module {path}: {err}"))),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn bool_from_value(cx: &mut Cx, value: Value, message: &'static str) -> Result<bool> {
    match value.object().as_expr(cx)? {
        Expr::Bool(value) => Ok(value),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn symbol_list_from_value(cx: &mut Cx, value: Value) -> Result<Vec<sim_kernel::Symbol>> {
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => items
            .into_iter()
            .map(|expr| match expr {
                Expr::Symbol(symbol) => Ok(symbol),
                _ => Err(Error::TypeMismatch {
                    expected: "symbol list",
                    found: "non-symbol",
                }),
            })
            .collect(),
        Expr::Symbol(symbol) => Ok(vec![symbol]),
        _ => Err(Error::TypeMismatch {
            expected: "symbol list",
            found: "non-list",
        }),
    }
}

pub(crate) fn capability_names_from_value(
    cx: &mut Cx,
    value: Value,
) -> Result<Vec<CapabilityName>> {
    capability_names_from_expr(&value.object().as_expr(cx)?)
}

pub(crate) fn clone_server_cx(seed: &Cx) -> Cx {
    seed.fork_from_seed(HandleSeed::new(1))
}

pub(crate) fn coerce_result_shape(
    cx: &mut Cx,
    value: Value,
) -> Result<Option<sim_kernel::ShapeRef>> {
    if matches!(value.object().as_expr(cx)?, Expr::Nil) {
        return Ok(None);
    }
    if value.object().as_shape().is_some() {
        return Ok(Some(value));
    }
    if let Some(class) = value.object().as_class() {
        return Ok(Some(class.instance_shape(cx)?));
    }
    Err(Error::TypeMismatch {
        expected: "shape or class",
        found: "non-shape",
    })
}

pub(crate) fn server_arg<'a>(
    args: &'a Args,
    index: usize,
    message: &'static str,
) -> Result<&'a Server> {
    let Some(value) = args.values().get(index) else {
        return Err(Error::Eval(message.to_owned()));
    };
    value
        .object()
        .downcast_ref::<Server>()
        .ok_or(Error::TypeMismatch {
            expected: "server",
            found: "non-server",
        })
}

pub(crate) fn connection_arg<'a>(
    args: &'a Args,
    index: usize,
    message: &'static str,
) -> Result<&'a Connection> {
    let Some(value) = args.values().get(index) else {
        return Err(Error::Eval(message.to_owned()));
    };
    value
        .object()
        .downcast_ref::<Connection>()
        .ok_or(Error::TypeMismatch {
            expected: "connection",
            found: "non-connection",
        })
}

#[cfg(test)]
mod fork_tests {
    use super::clone_server_cx;
    use sim_kernel::{CapabilityName, testing::bare_cx};

    #[test]
    fn clone_server_cx_preserves_seed_capabilities() {
        let mut seed = bare_cx();
        let capability = CapabilityName::new("test.server-fork");
        seed.grant(capability.clone());
        let fork = clone_server_cx(&seed);
        assert!(fork.capabilities().contains(&capability));
    }
}
