use crate::Tool;
use sim_codec_chat::{is_model_request_expr, validate_chat_transcript};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_value::access::field;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn is_model_request(expr: &Expr) -> bool {
    is_model_request_expr(expr)
}

pub(crate) fn inject_manifest_tools(cx: &mut Cx, request: Expr, tools: &[Value]) -> Result<Expr> {
    if tools.is_empty() && field(&request, "tools").is_none() {
        return Ok(request);
    }
    if !is_model_request(&request) {
        return Ok(request);
    }
    let manifest_tools = manifest_tool_descriptors(cx, tools)?;
    let allowed = allowed_tools(&request, &manifest_tools)?;
    let mut entries = request_map_entries(request)?;
    let mut tool_items = remove_field(&mut entries, "tools")
        .map(tools_list)
        .transpose()?
        .unwrap_or_default();
    let mut declared = BTreeSet::new();
    for item in &tool_items {
        let symbol = required_descriptor_symbol(item)?;
        if !manifest_tools.contains_key(&symbol) {
            return Err(Error::Eval(format!(
                "tool {symbol} is not declared by this agent"
            )));
        }
        if !allowed.contains(&symbol) {
            return Err(Error::Eval(format!(
                "tool {symbol} is denied by request policy"
            )));
        }
        if let Some(generated) = manifest_tools.get(&symbol) {
            reject_conflicting_descriptor(item, generated, &symbol)?;
        }
        declared.insert(symbol);
    }
    let mut injected = Vec::new();
    for symbol in &allowed {
        if declared.contains(symbol) {
            continue;
        }
        if let Some(descriptor) = manifest_tools.get(symbol) {
            tool_items.push(descriptor.clone());
            injected.push((symbol.clone(), descriptor.clone()));
        }
    }
    if !tool_items.is_empty() {
        entries.push(key_expr("tools", Expr::List(tool_items)));
    }
    if !injected.is_empty() {
        entries.push(key_expr(
            "agent-tool-injection",
            injection_metadata(injected),
        ));
    }
    let out = Expr::Map(entries);
    validate_chat_transcript(&out)?;
    Ok(out)
}

fn manifest_tool_descriptors(cx: &mut Cx, tools: &[Value]) -> Result<BTreeMap<Symbol, Expr>> {
    let mut descriptors = BTreeMap::new();
    for value in tools {
        let tool = value
            .object()
            .downcast_ref::<Tool>()
            .ok_or(Error::TypeMismatch {
                expected: "tool",
                found: "non-tool",
            })?;
        if descriptors
            .insert(tool.symbol.clone(), tool.model_descriptor_expr(cx)?)
            .is_some()
        {
            return Err(Error::Eval(format!(
                "agent manifest contains duplicate tool {}",
                tool.symbol
            )));
        }
    }
    Ok(descriptors)
}

fn allowed_tools(request: &Expr, manifest: &BTreeMap<Symbol, Expr>) -> Result<BTreeSet<Symbol>> {
    let policy = ToolRequestPolicy::from_request(request)?;
    let mut allowed = manifest.keys().cloned().collect::<BTreeSet<_>>();
    if let Some(allow) = policy.allow {
        allowed.retain(|symbol| allow.contains(symbol));
    }
    allowed.retain(|symbol| !policy.deny.contains(symbol));
    Ok(allowed)
}

#[derive(Default)]
struct ToolRequestPolicy {
    allow: Option<BTreeSet<Symbol>>,
    deny: BTreeSet<Symbol>,
}

impl ToolRequestPolicy {
    fn from_request(request: &Expr) -> Result<Self> {
        let mut policy = Self::default();
        if let Some(allow) = field(request, "allowed-tools") {
            policy.allow = Some(symbol_set(allow, "allowed-tools")?);
        }
        if let Some(deny) = field(request, "denied-tools") {
            policy.deny.extend(symbol_set(deny, "denied-tools")?);
        }
        let Some(tool_policy) = field(request, "tool-policy") else {
            return Ok(policy);
        };
        if let Some(allow) = field(tool_policy, "allow").or_else(|| field(tool_policy, "tools")) {
            policy.allow = Some(symbol_set(allow, "tool-policy allow")?);
        }
        if let Some(deny) = field(tool_policy, "deny") {
            policy.deny.extend(symbol_set(deny, "tool-policy deny")?);
        }
        Ok(policy)
    }
}

fn request_map_entries(request: Expr) -> Result<Vec<(Expr, Expr)>> {
    match request {
        Expr::Map(entries) => Ok(entries),
        _ => Err(Error::Eval("model request must be a map".to_owned())),
    }
}

fn remove_field(entries: &mut Vec<(Expr, Expr)>, name: &str) -> Option<Expr> {
    let index = entries.iter().position(|(key, _)| is_field(key, name))?;
    Some(entries.remove(index).1)
}

fn tools_list(expr: Expr) -> Result<Vec<Expr>> {
    match expr {
        Expr::List(items) | Expr::Vector(items) => Ok(items),
        _ => Err(Error::Eval(
            "model request tools field must be a list".to_owned(),
        )),
    }
}

fn reject_conflicting_descriptor(explicit: &Expr, generated: &Expr, symbol: &Symbol) -> Result<()> {
    let Expr::Map(entries) = explicit else {
        return Ok(());
    };
    for (key, value) in entries {
        let Some(name) = key_name(key) else {
            continue;
        };
        if matches!(name, "name" | "tool" | "function") {
            continue;
        }
        if let Some(generated_value) = field(generated, name)
            && generated_value != value
        {
            return Err(Error::Eval(format!(
                "tool descriptor for {symbol} conflicts on field {name}"
            )));
        }
    }
    Ok(())
}

fn injection_metadata(injected: Vec<(Symbol, Expr)>) -> Expr {
    let symbols = injected
        .iter()
        .map(|(symbol, _)| Expr::Symbol(symbol.clone()))
        .collect();
    let descriptors = injected
        .into_iter()
        .map(|(_, descriptor)| descriptor)
        .collect();
    Expr::Map(vec![
        key_expr("source", Expr::Symbol(Symbol::new("agent-manifest"))),
        key_expr("injected-tools", Expr::List(symbols)),
        key_expr("descriptors", Expr::List(descriptors)),
    ])
}

fn required_descriptor_symbol(expr: &Expr) -> Result<Symbol> {
    descriptor_symbol(expr)?.ok_or_else(|| Error::Eval("tool descriptor missing name".to_owned()))
}

fn descriptor_symbol(expr: &Expr) -> Result<Option<Symbol>> {
    match expr {
        Expr::Symbol(symbol) => Ok(Some(symbol.clone())),
        Expr::String(text) => Ok(Some(symbol_from_text(text))),
        Expr::Map(_) => {
            if let Some(name) = field(expr, "name").or_else(|| field(expr, "tool")) {
                return symbol_from_expr(name, "tool descriptor name must be a symbol or string")
                    .map(Some);
            }
            if let Some(function) = field(expr, "function")
                && let Some(name) = field(function, "name")
            {
                return symbol_from_expr(
                    name,
                    "tool descriptor function name must be a symbol or string",
                )
                .map(Some);
            }
            Ok(None)
        }
        _ => Err(Error::Eval(
            "tool descriptor must be a symbol, string, or map".to_owned(),
        )),
    }
}

fn symbol_set(expr: &Expr, label: &str) -> Result<BTreeSet<Symbol>> {
    let items = match expr {
        Expr::Nil => Vec::new(),
        Expr::List(items) | Expr::Vector(items) => items.clone(),
        other => vec![other.clone()],
    };
    items
        .iter()
        .map(|item| symbol_from_expr(item, &format!("{label} expects symbols or strings")))
        .collect()
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

fn key_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.as_ref()),
        _ => None,
    }
}

fn is_field(expr: &Expr, name: &str) -> bool {
    matches!(key_name(expr), Some(found) if found == name)
}

use sim_value::build::entry as key_expr;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_field_keeps_bare_symbol_policy() {
        let map = Expr::Map(vec![
            key_expr("bare", Expr::String("accepted".to_owned())),
            (
                Expr::String("string".to_owned()),
                Expr::String("rejected".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::qualified("provider", "qualified")),
                Expr::String("rejected".to_owned()),
            ),
        ]);

        assert!(matches!(
            field(&map, "bare"),
            Some(Expr::String(value)) if value == "accepted"
        ));
        assert_eq!(field(&map, "string"), None);
        assert_eq!(field(&map, "qualified"), None);
    }

    #[test]
    fn descriptor_key_names_are_bare_symbols_only() {
        let cases = [
            (Expr::Symbol(Symbol::new("bare")), Some("bare")),
            (Expr::String("string".to_owned()), None),
            (Expr::Symbol(Symbol::qualified("ns", "name")), None),
        ];

        for (expr, expected) in cases {
            assert_eq!(key_name(&expr), expected);
        }
    }
}
