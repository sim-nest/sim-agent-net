use crate::util::{
    f64_from_value, keyword, parse_capabilities_expr, string_from_value, stringish_from_value,
    symbol_from_value, symbol_of, u32_from_value,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, Result, Symbol, Value};
#[cfg(feature = "runner-process")]
use sim_shape::{OptionFieldSpec, TableExtraPolicy};
use std::{collections::HashMap, path::PathBuf};

pub(crate) fn parse_component_options(
    cx: &mut Cx,
    args: Args,
    name: &'static str,
) -> Result<HashMap<String, Value>> {
    if !args.values().len().is_multiple_of(2) {
        return Err(Error::Eval(format!(
            "{name} options must be key/value pairs"
        )));
    }
    let mut options = HashMap::new();
    for pair in args.values().chunks(2) {
        let key = keyword(&pair[0].object().as_expr(cx)?)?;
        options.insert(key, pair[1].clone());
    }
    Ok(options)
}

pub(crate) fn maybe_u32_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<u32>> {
    options
        .get(key)
        .map(|value| u32_from_value(cx, value.clone(), "expected integer option"))
        .transpose()
}

pub(crate) fn maybe_f64_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<f64>> {
    options
        .get(key)
        .map(|value| f64_from_value(cx, value.clone(), "expected numeric option"))
        .transpose()
}

pub(crate) fn string_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: &str,
) -> Result<String> {
    match options.get(key) {
        Some(value) => stringish_from_value(cx, value.clone(), "expected string or symbol option"),
        None => Ok(default.to_owned()),
    }
}

pub(crate) fn expr_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<Expr>> {
    options
        .get(key)
        .map(|value| value.object().as_expr(cx))
        .transpose()
}

pub(crate) fn symbol_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: Symbol,
) -> Result<Symbol> {
    match options.get(key) {
        Some(value) => symbol_from_value(cx, value.clone(), "expected symbol option"),
        None => Ok(default),
    }
}

pub(crate) fn capabilities_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<CapabilityName>> {
    match options.get(key) {
        Some(value) => parse_capabilities_expr(&value.object().as_expr(cx)?),
        None => Ok(Vec::new()),
    }
}

pub(crate) fn symbols_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<Symbol>> {
    let Some(value) = options.get(key) else {
        return Ok(Vec::new());
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => items
            .into_iter()
            .map(|expr| symbol_of(&expr, "expected symbol list item"))
            .collect(),
        expr => Ok(vec![symbol_of(&expr, "expected symbol or symbol list")?]),
    }
}

pub(crate) fn path_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<PathBuf>> {
    match options.get(key) {
        Some(value) => Ok(Some(PathBuf::from(string_from_value(
            cx,
            value.clone(),
            "expected string path option",
        )?))),
        None => Ok(None),
    }
}

pub(crate) fn strings_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<String>> {
    let Some(value) = options.get(key) else {
        return Ok(Vec::new());
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => items
            .iter()
            .map(|expr| match expr {
                Expr::String(text) => Ok(text.clone()),
                Expr::Symbol(symbol) => Ok(symbol.to_string()),
                _ => Err(Error::Eval("expected string corpus item".to_owned())),
            })
            .collect(),
        Expr::String(text) => Ok(vec![text]),
        Expr::Symbol(symbol) => Ok(vec![symbol.to_string()]),
        _ => Err(Error::Eval(
            "expected string or string list option".to_owned(),
        )),
    }
}

#[cfg(feature = "runner-process")]
pub(crate) fn check_component_option_map(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    fields: Vec<OptionFieldSpec>,
    extra: TableExtraPolicy,
    name: &'static str,
) -> Result<()> {
    let expr = options_expr(cx, options)?;
    let matched = sim_shape::check_option_map(cx, &expr, fields, extra)?;
    if matched.accepted {
        return Ok(());
    }
    let message = matched
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "option shape rejected".to_owned());
    Err(Error::Eval(format!("{name} options rejected: {message}")))
}

#[cfg(feature = "runner-process")]
fn options_expr(cx: &mut Cx, options: &HashMap<String, Value>) -> Result<Expr> {
    options
        .iter()
        .map(|(key, value)| {
            Ok((
                Expr::Symbol(Symbol::new(key.clone())),
                value.object().as_expr(cx)?,
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(Expr::Map)
}
