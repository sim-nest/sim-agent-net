use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketPolicy {
    pub(crate) prefer: Symbol,
    pub(crate) execution: Option<Symbol>,
    pub(crate) quorum: Option<u64>,
    pub(crate) max_cost_usd: Option<f64>,
    pub(crate) deadline: Option<String>,
    pub(crate) min_context_tokens: Option<u64>,
    pub(crate) requires: Vec<Symbol>,
    pub(crate) fallback: Option<Symbol>,
    pub(crate) verify_with: Option<Symbol>,
    pub(crate) judge: Option<Symbol>,
}

impl Default for MarketPolicy {
    fn default() -> Self {
        Self {
            prefer: Symbol::new("local-first"),
            execution: None,
            quorum: None,
            max_cost_usd: None,
            deadline: None,
            min_context_tokens: None,
            requires: Vec::new(),
            fallback: None,
            verify_with: None,
            judge: None,
        }
    }
}

impl MarketPolicy {
    pub(crate) fn from_options(cx: &mut Cx, options: &HashMap<String, Value>) -> Result<Self> {
        Ok(Self {
            prefer: optional_symbol(cx, options, "prefer")?.unwrap_or(Symbol::new("local-first")),
            execution: optional_symbol(cx, options, "execution")?
                .or(optional_symbol(cx, options, "mode")?),
            quorum: optional_u64(cx, options, "quorum")?,
            max_cost_usd: optional_f64(cx, options, "max-cost-usd")?,
            deadline: optional_string(cx, options, "deadline")?,
            min_context_tokens: optional_u64(cx, options, "min-context-tokens")?,
            requires: optional_symbols(cx, options, "requires")?,
            fallback: optional_symbol(cx, options, "fallback")?,
            verify_with: optional_symbol(cx, options, "verify-with")?,
            judge: optional_symbol(cx, options, "judge")?,
        })
    }

    pub(crate) fn from_expr(expr: &Expr) -> Result<Self> {
        let mut policy = Self::default();
        if let Some(value) = field(expr, "prefer") {
            policy.prefer = expr_symbol(value, "model-policy :prefer expects a symbol")?;
        }
        policy.execution = field(expr, "execution")
            .or_else(|| field(expr, "mode"))
            .map(|expr| expr_symbol(expr, "model-policy :execution expects a symbol"))
            .transpose()?;
        policy.quorum = field(expr, "quorum").map(require_u64).transpose()?;
        policy.max_cost_usd = field(expr, "max-cost-usd").map(require_f64).transpose()?;
        policy.deadline = field(expr, "deadline").map(expr_label).transpose()?;
        policy.min_context_tokens = field(expr, "min-context-tokens")
            .map(require_u64)
            .transpose()?;
        policy.requires = field(expr, "requires")
            .map(expr_symbols)
            .transpose()?
            .unwrap_or_default();
        policy.fallback = field(expr, "fallback")
            .map(|expr| expr_symbol(expr, "model-policy :fallback expects a symbol"))
            .transpose()?;
        policy.verify_with = field(expr, "verify-with")
            .map(|expr| expr_symbol(expr, "model-policy :verify-with expects a symbol"))
            .transpose()?;
        policy.judge = field(expr, "judge")
            .map(|expr| expr_symbol(expr, "model-policy :judge expects a symbol"))
            .transpose()?;
        Ok(policy)
    }

    pub(crate) fn to_expr(&self) -> Expr {
        let mut entries = vec![key_expr("prefer", Expr::Symbol(self.prefer.clone()))];
        if let Some(value) = &self.execution {
            entries.push(key_expr("execution", Expr::Symbol(value.clone())));
        }
        if let Some(value) = self.quorum {
            entries.push(key_expr("quorum", number_expr(value)));
        }
        if let Some(value) = self.max_cost_usd {
            entries.push(key_expr("max-cost-usd", float_expr(value)));
        }
        if let Some(value) = &self.deadline {
            entries.push(key_expr("deadline", Expr::String(value.clone())));
        }
        if let Some(value) = self.min_context_tokens {
            entries.push(key_expr("min-context-tokens", number_expr(value)));
        }
        if !self.requires.is_empty() {
            entries.push(key_expr(
                "requires",
                Expr::List(self.requires.iter().cloned().map(Expr::Symbol).collect()),
            ));
        }
        if let Some(value) = &self.fallback {
            entries.push(key_expr("fallback", Expr::Symbol(value.clone())));
        }
        if let Some(value) = &self.verify_with {
            entries.push(key_expr("verify-with", Expr::Symbol(value.clone())));
        }
        if let Some(value) = &self.judge {
            entries.push(key_expr("judge", Expr::Symbol(value.clone())));
        }
        Expr::Map(entries)
    }
}

pub(crate) fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}

pub(crate) fn expr_symbols(expr: &Expr) -> Result<Vec<Symbol>> {
    match expr {
        Expr::Nil => Ok(Vec::new()),
        Expr::Symbol(symbol) => Ok(vec![symbol.clone()]),
        Expr::String(text) => Ok(vec![Symbol::new(text.clone())]),
        Expr::List(items) | Expr::Vector(items) => items
            .iter()
            .map(|item| expr_symbol(item, "expected symbol list item"))
            .collect(),
        _ => Err(Error::Eval("expected symbol or symbol list".to_owned())),
    }
}

pub(crate) fn expr_label(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval("expected string or symbol".to_owned())),
    }
}

pub(crate) fn expr_f64(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(number) => number.canonical.parse::<f64>().ok(),
        _ => None,
    }
}

pub(crate) fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

pub(crate) use sim_value::build::entry as key_expr;

pub(crate) fn float_expr(value: f64) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn optional_f64(cx: &mut Cx, options: &HashMap<String, Value>, key: &str) -> Result<Option<f64>> {
    options
        .get(key)
        .map(|value| require_f64(&value.object().as_expr(cx)?))
        .transpose()
}

fn optional_u64(cx: &mut Cx, options: &HashMap<String, Value>, key: &str) -> Result<Option<u64>> {
    options
        .get(key)
        .map(|value| require_u64(&value.object().as_expr(cx)?))
        .transpose()
}

fn optional_string(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<String>> {
    options
        .get(key)
        .map(|value| expr_label(&value.object().as_expr(cx)?))
        .transpose()
}

fn optional_symbol(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<Symbol>> {
    options
        .get(key)
        .map(|value| expr_symbol(&value.object().as_expr(cx)?, "expected symbol option"))
        .transpose()
}

fn optional_symbols(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<Symbol>> {
    options
        .get(key)
        .map(|value| expr_symbols(&value.object().as_expr(cx)?))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn expr_symbol(expr: &Expr, message: &'static str) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        Expr::String(text) => Ok(Symbol::new(text.clone())),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

fn require_f64(expr: &Expr) -> Result<f64> {
    expr_f64(expr).ok_or_else(|| Error::Eval("expected numeric option".to_owned()))
}

fn require_u64(expr: &Expr) -> Result<u64> {
    expr_u64(expr).ok_or_else(|| Error::Eval("expected integer option".to_owned()))
}

fn expr_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Number(number) => number.canonical.parse::<u64>().ok(),
        Expr::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn number_expr(value: u64) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_field_reader_keeps_symbol_namespace_policy() {
        let map = Expr::Map(vec![
            key_expr("bare", Expr::String("bare".to_owned())),
            (
                Expr::Symbol(Symbol::qualified("provider", "qualified")),
                Expr::String("qualified".to_owned()),
            ),
            (
                Expr::String("string".to_owned()),
                Expr::String("string".to_owned()),
            ),
        ]);

        assert!(matches!(
            field(&map, "bare"),
            Some(Expr::String(value)) if value == "bare"
        ));
        assert!(matches!(
            field(&map, "qualified"),
            Some(Expr::String(value)) if value == "qualified"
        ));
        assert_eq!(field(&map, "string"), None);
    }
}
