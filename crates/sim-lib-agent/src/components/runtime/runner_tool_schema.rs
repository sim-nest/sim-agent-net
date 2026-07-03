use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Result, Symbol};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(super) struct ToolCall {
    pub(super) id: Expr,
    pub(super) name: Symbol,
    pub(super) args: Vec<Expr>,
    pub(super) raw: Expr,
}

pub(super) fn tool_calls_from_response(response: &Expr) -> Result<Vec<ToolCall>> {
    let Some(tool_calls) = field(response, "tool-calls") else {
        return Ok(Vec::new());
    };
    let Expr::List(items) = tool_calls else {
        return Err(Error::Eval(
            "model response tool-calls field must be a list".to_owned(),
        ));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| tool_call_from_expr(item, index))
        .collect()
}

pub(super) fn append_tool_messages(request: Expr, messages: Vec<Expr>) -> Result<Expr> {
    let Expr::Map(mut entries) = request else {
        return Err(Error::Eval(
            "tool-call continuation requires a model-request map".to_owned(),
        ));
    };
    let mut inserted = false;
    for (key, value) in &mut entries {
        if is_field(key, "messages") {
            let Expr::List(items) = value else {
                return Err(Error::Eval(
                    "model request messages field must be a list".to_owned(),
                ));
            };
            items.extend(messages.clone());
            inserted = true;
            break;
        }
    }
    if !inserted {
        entries.push((Expr::Symbol(Symbol::new("messages")), Expr::List(messages)));
    }
    let out = Expr::Map(entries);
    validate_chat_transcript(&out)?;
    Ok(out)
}

pub(super) fn declared_tools(request: &Expr) -> Result<Option<BTreeSet<Symbol>>> {
    let Some(tools) = field(request, "tools") else {
        return Ok(None);
    };
    let Expr::List(items) = tools else {
        return Err(Error::Eval(
            "model request tools field must be a list".to_owned(),
        ));
    };
    let mut declared = BTreeSet::new();
    for item in items {
        if let Some(symbol) = tool_descriptor_symbol(item)? {
            declared.insert(symbol);
        }
    }
    Ok(Some(declared))
}

pub(super) fn max_tool_rounds(request: &Expr) -> Result<u32> {
    if let Some(value) = field(request, "max-tool-rounds") {
        return u32_from_expr(value, "max-tool-rounds");
    }
    if let Some(budget) = field(request, "budget")
        && let Some(value) = field(budget, "max-tool-rounds")
    {
        return u32_from_expr(value, "budget max-tool-rounds");
    }
    Ok(4)
}

pub(super) fn response_runner(response: &Expr) -> Option<Symbol> {
    match field(response, "runner") {
        Some(Expr::Symbol(symbol)) => Some(symbol.clone()),
        _ => None,
    }
}

pub(super) fn response_model(response: &Expr) -> Option<String> {
    match field(response, "model") {
        Some(Expr::String(model)) => Some(model.clone()),
        _ => None,
    }
}

pub(super) fn tool_call_fingerprint(call: &ToolCall) -> String {
    format!("{}::{:?}", call.name, call.args)
}

pub(super) fn tool_result_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => format!("{expr:?}"),
    }
}

pub(super) fn bool_field(expr: &Expr, name: &str) -> Option<bool> {
    match field(expr, name) {
        Some(Expr::Bool(value)) => Some(*value),
        _ => None,
    }
}

pub(super) use sim_value::build::entry as key_expr;

fn tool_call_from_expr(expr: &Expr, index: usize) -> Result<ToolCall> {
    let Expr::Map(_) = expr else {
        return Err(Error::Eval("tool call must be a map".to_owned()));
    };
    let nested = field(expr, "function");
    let name_expr = field(expr, "name")
        .or_else(|| field(expr, "tool"))
        .or_else(|| nested.and_then(|expr| field(expr, "name")))
        .ok_or_else(|| Error::Eval("tool call missing name".to_owned()))?;
    let id = field(expr, "id")
        .or_else(|| field(expr, "tool-call-id"))
        .cloned()
        .unwrap_or_else(|| Expr::String(format!("tool-call-{index}")));
    let args_expr = field(expr, "arguments")
        .or_else(|| field(expr, "args"))
        .or_else(|| nested.and_then(|expr| field(expr, "arguments")));
    Ok(ToolCall {
        id,
        name: symbol_from_expr(name_expr, "tool call name must be a symbol or string")?,
        args: args_from_expr(args_expr),
        raw: expr.clone(),
    })
}

fn args_from_expr(expr: Option<&Expr>) -> Vec<Expr> {
    match expr {
        None | Some(Expr::Nil) => Vec::new(),
        Some(Expr::List(items)) | Some(Expr::Vector(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    }
}

fn tool_descriptor_symbol(expr: &Expr) -> Result<Option<Symbol>> {
    match expr {
        Expr::Symbol(symbol) => Ok(Some(symbol.clone())),
        Expr::String(text) => Ok(Some(symbol_from_text(text))),
        Expr::Map(_) => {
            if let Some(name) = field(expr, "name").or_else(|| field(expr, "tool")) {
                return Ok(Some(symbol_from_expr(
                    name,
                    "tool descriptor name must be a symbol or string",
                )?));
            }
            if let Some(function) = field(expr, "function")
                && let Some(name) = field(function, "name")
            {
                return Ok(Some(symbol_from_expr(
                    name,
                    "tool descriptor function name must be a symbol or string",
                )?));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn u32_from_expr(expr: &Expr, field_name: &str) -> Result<u32> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse()
            .map_err(|_| Error::Eval(format!("{field_name} must be an integer"))),
        _ => Err(Error::Eval(format!("{field_name} must be a number"))),
    }
}

fn symbol_from_expr(expr: &Expr, error: &str) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        Expr::String(text) => Ok(symbol_from_text(text)),
        _ => Err(Error::Eval(error.to_owned())),
    }
}

fn symbol_from_text(text: &str) -> Symbol {
    match text.split_once('/') {
        Some((namespace, name)) => Symbol::qualified(namespace.to_owned(), name.to_owned()),
        None => Symbol::new(text.to_owned()),
    }
}

use sim_value::access::field;

fn is_field(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name)
}
