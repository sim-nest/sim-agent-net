use super::market_policy::{expr_label, expr_symbols, key_bool, key_expr};
use super::model::{AgentComponent, ComponentBackend, RunnerBackend};
use crate::ComponentKind;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_agent_runner_core::{ModelBid, ModelCard, ModelRequest, ModelResponse};

pub(crate) fn runner_card(_cx: &mut Cx, runner: &Value) -> Result<ModelCard> {
    let component = runner_component(runner)?;
    if component.kind != ComponentKind::Runner {
        return Err(Error::Eval(format!("{} is not a runner", component.symbol)));
    }
    match &component.backend {
        ComponentBackend::Runner(RunnerBackend::Echo { model }) => {
            builtin_card(component, model.as_str(), "echo")
        }
        ComponentBackend::Runner(RunnerBackend::Cassette { model, .. }) => {
            builtin_card(component, model.as_str(), "cassette")
        }
        ComponentBackend::Runner(RunnerBackend::Fake { model, .. }) => {
            builtin_card(component, model.as_str(), "fake")
        }
        ComponentBackend::Runner(RunnerBackend::External { runner }) => {
            let mut card = runner.card();
            card.extra
                .extend(component.spec.iter().filter_map(card_extra));
            if card.runner != component.symbol {
                card.extra.push(key_expr(
                    "reported-runner",
                    Expr::Symbol(card.runner.clone()),
                ));
                card.runner = component.symbol.clone();
            }
            Ok(card)
        }
        _ => Err(Error::Eval(format!("{} is not a runner", component.symbol))),
    }
}

pub(crate) fn runner_bid(runner: &Value, request: &ModelRequest, card: &ModelCard) -> ModelBid {
    let external_bid =
        runner_component(runner)
            .ok()
            .and_then(|component| match &component.backend {
                ComponentBackend::Runner(RunnerBackend::External { runner }) => {
                    runner.bid(request).ok()
                }
                _ => None,
            });
    match external_bid {
        Some(bid) if bid.available => bid,
        Some(bid) if bid.reason.as_deref() != Some("runner did not implement bidding") => bid,
        _ => derived_bid(card),
    }
}

pub(crate) fn health_expr(card: &ModelCard) -> Expr {
    let mut entries = vec![
        key_expr("runner", Expr::Symbol(card.runner.clone())),
        key_bool("healthy", card_is_healthy(card)),
        key_expr("health", health_status(card)),
        key_expr("card", Expr::from(card.clone())),
    ];
    if let Some(reason) = card_reason(card) {
        entries.push(key_expr("reason", Expr::String(reason)));
    }
    Expr::Map(entries)
}

pub(crate) fn card_is_healthy(card: &ModelCard) -> bool {
    !matches!(extra_field(card, "healthy"), Some(Expr::Bool(false)))
        && !matches!(
            extra_field(card, "health"),
            Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "unhealthy"
        )
}

pub(crate) fn health_status(card: &ModelCard) -> Expr {
    extra_field(card, "health").cloned().unwrap_or_else(|| {
        Expr::Symbol(Symbol::new(if card_is_healthy(card) {
            "healthy"
        } else {
            "unhealthy"
        }))
    })
}

pub(crate) fn card_cost(card: &ModelCard) -> Option<f64> {
    extra_field(card, "cost-usd").and_then(expr_f64)
}

pub(crate) fn card_context_tokens(card: &ModelCard) -> Option<u64> {
    extra_field(card, "context-tokens").and_then(expr_u64)
}

pub(crate) fn card_supports(card: &ModelCard, requirement: &Symbol) -> bool {
    match requirement.name.as_ref() {
        "text" => true,
        "tools" => matches!(extra_field(card, "supports-tools"), Some(Expr::Bool(true))),
        "stream" => matches!(extra_field(card, "supports-stream"), Some(Expr::Bool(true))),
        "json" => matches!(extra_field(card, "supports-json"), Some(Expr::Bool(true))),
        "shape" => matches!(extra_field(card, "supports-shape"), Some(Expr::Bool(true))),
        _ => {
            list_field_contains(card, "requires", requirement)
                || list_field_contains(card, "modalities", requirement)
        }
    }
}

pub(crate) fn is_retryable_error(response: &ModelResponse) -> bool {
    response.stop_reason.name.as_ref() == "error"
        && response.extra.iter().any(|(key, value)| {
            matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "retryable")
                && matches!(value, Expr::Bool(true))
        })
}

pub(crate) fn runner_name_expr(value: &Value) -> Expr {
    runner_component(value)
        .map(|component| Expr::Symbol(component.symbol.clone()))
        .unwrap_or(Expr::Nil)
}

pub(crate) fn runner_symbol(value: &Value) -> Option<Symbol> {
    runner_component(value)
        .ok()
        .map(|component| component.symbol.clone())
}

pub(crate) fn expr_f64(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(number) => number.canonical.parse::<f64>().ok(),
        _ => None,
    }
}

fn runner_component(value: &Value) -> Result<&AgentComponent> {
    value
        .object()
        .downcast_ref::<AgentComponent>()
        .ok_or_else(|| Error::Eval("expected runner component".to_owned()))
}

fn builtin_card(component: &AgentComponent, model: &str, provider: &str) -> Result<ModelCard> {
    let mut card = ModelCard::new(
        component.symbol.clone(),
        model,
        Symbol::new(provider),
        Symbol::new("local"),
    );
    card.extra
        .extend(component.spec.iter().filter_map(card_extra));
    Ok(card)
}

fn derived_bid(card: &ModelCard) -> ModelBid {
    let healthy = card_is_healthy(card);
    ModelBid {
        available: healthy,
        reason: (!healthy)
            .then(|| card_reason(card).unwrap_or_else(|| "runner reported unhealthy".to_owned())),
        score: card_cost(card),
        model: Some(card.model.clone()),
        extra: Vec::new(),
    }
}

fn card_extra((key, value): &(Symbol, Expr)) -> Option<(Expr, Expr)> {
    matches!(
        key.name.as_ref(),
        "cost-usd"
            | "context-tokens"
            | "healthy"
            | "health"
            | "health-reason"
            | "requires"
            | "modalities"
            | "modalities-in"
            | "modalities-out"
            | "privacy"
            | "quality"
            | "latency-ms"
            | "delay-ms"
            | "supports-stream"
            | "supports-tools"
            | "supports-json"
            | "supports-shape"
            | "agent"
            | "timeout"
    )
    .then(|| (Expr::Symbol(key.clone()), value.clone()))
}

fn card_reason(card: &ModelCard) -> Option<String> {
    extra_field(card, "health-reason").and_then(|expr| expr_label(expr).ok())
}

fn list_field_contains(card: &ModelCard, key: &str, wanted: &Symbol) -> bool {
    extra_field(card, key).is_some_and(|expr| {
        expr_symbols(expr).is_ok_and(|items| items.iter().any(|item| item.name == wanted.name))
    })
}

fn extra_field<'a>(card: &'a ModelCard, name: &str) -> Option<&'a Expr> {
    card.extra.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}

fn expr_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Number(number) => number.canonical.parse::<u64>().ok(),
        Expr::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}
