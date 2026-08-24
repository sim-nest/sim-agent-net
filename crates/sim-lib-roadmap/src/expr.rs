use crate::{RoadmapValue, RoadmapValueKind, RoadmapValueLimits};
use sim_kernel::{Error, Expr, Result, Symbol};
use std::collections::BTreeMap;

/// Stable extension tag for every roadmap-family value.
pub fn roadmap_value_symbol() -> Symbol {
    Symbol::qualified("roadmap", "Value")
}

/// Project a validated value into the shared expression graph.
pub fn roadmap_value_to_expr(value: &RoadmapValue) -> Expr {
    let mut fields = vec![
        (
            Expr::Symbol(Symbol::new("kind")),
            Expr::String(value.kind().wire_name().into()),
        ),
        (
            Expr::Symbol(Symbol::new("semantic-id")),
            Expr::String(value.semantic_id().into()),
        ),
    ];
    fields.extend(
        value
            .fields()
            .iter()
            .map(|(k, v)| (Expr::Symbol(k.clone()), v.clone())),
    );
    Expr::Extension {
        tag: roadmap_value_symbol(),
        payload: Box::new(Expr::Map(fields)),
    }
}

/// Strictly recover a value, checking bounds and its claimed semantic id.
pub fn roadmap_value_from_expr(expr: &Expr) -> Result<RoadmapValue> {
    roadmap_value_from_expr_with_limits(expr, RoadmapValueLimits::default())
}

pub fn roadmap_value_from_expr_with_limits(
    expr: &Expr,
    limits: RoadmapValueLimits,
) -> Result<RoadmapValue> {
    let Expr::Extension { tag, payload } = expr else {
        return Err(Error::Eval("expected tagged roadmap value".into()));
    };
    if tag != &roadmap_value_symbol() {
        return Err(Error::Eval("unexpected roadmap value tag".into()));
    }
    let Expr::Map(entries) = payload.as_ref() else {
        return Err(Error::Eval("roadmap value payload must be a map".into()));
    };
    if entries.len() > limits.fields.saturating_add(2) {
        return Err(Error::Eval("roadmap value has too many fields".into()));
    }
    let mut map = BTreeMap::new();
    for (key, value) in entries {
        let Expr::Symbol(key) = key else {
            return Err(Error::Eval("roadmap field key must be a symbol".into()));
        };
        if map.insert(key.clone(), value.clone()).is_some() {
            return Err(Error::Eval(format!("duplicate roadmap field {key}")));
        }
    }
    let kind = match map.remove(&Symbol::new("kind")) {
        Some(Expr::String(name)) => RoadmapValueKind::parse(&name)
            .ok_or_else(|| Error::Eval(format!("unknown roadmap value kind {name}")))?,
        _ => return Err(Error::Eval("roadmap value requires string kind".into())),
    };
    let claimed = match map.remove(&Symbol::new("semantic-id")) {
        Some(Expr::String(id)) => id,
        _ => return Err(Error::Eval("roadmap value requires semantic-id".into())),
    };
    let value = RoadmapValue::with_limits(kind, map, limits)?;
    if value.semantic_id() != claimed {
        return Err(Error::Eval("forged roadmap semantic-id".into()));
    }
    Ok(value)
}
