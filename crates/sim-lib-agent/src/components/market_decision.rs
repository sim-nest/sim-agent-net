use super::market_cards::{card_cost, health_status};
use super::market_policy::{MarketPolicy, expr_f64, field, float_expr, key_bool, key_expr};
use crate::model_privacy::PrivacyPolicy;
use sim_kernel::{Expr, Symbol, Value};
use sim_lib_agent_runner_core::{ModelBid, ModelCard, ModelRequest, ModelResponse};

#[derive(Clone)]
pub(super) struct MarketCandidate {
    pub(super) runner: Value,
    pub(super) card: ModelCard,
    pub(super) bid: ModelBid,
    pub(super) score: f64,
}

impl MarketCandidate {
    pub(super) fn symbol(&self) -> Symbol {
        self.card.runner.clone()
    }

    pub(super) fn latency_ms(&self) -> u64 {
        extra_field(&self.card, "latency-ms")
            .or_else(|| extra_field(&self.card, "delay-ms"))
            .and_then(expr_u64)
            .unwrap_or(0)
    }
}

pub(super) struct MarketDecision {
    policy: MarketPolicy,
    pub(super) privacy: PrivacyPolicy,
    bids: Vec<Expr>,
    candidates: Vec<Expr>,
    runs: Vec<Expr>,
    cancellations: Vec<Expr>,
    pub(super) fallback_used: Option<Symbol>,
    pub(super) verified_by: Option<Symbol>,
    pub(super) selected: Option<Symbol>,
    pub(super) selected_response: Option<Expr>,
    failures: Vec<Expr>,
}

impl MarketDecision {
    pub(super) fn new(policy: MarketPolicy, privacy: PrivacyPolicy) -> Self {
        Self {
            policy,
            privacy,
            bids: Vec::new(),
            candidates: Vec::new(),
            runs: Vec::new(),
            cancellations: Vec::new(),
            fallback_used: None,
            verified_by: None,
            selected: None,
            selected_response: None,
            failures: Vec::new(),
        }
    }

    pub(super) fn privacy_rejected(&mut self, card: &ModelCard, reason: String) {
        self.bids.push(Expr::Map(vec![
            key_expr("runner", Expr::Symbol(card.runner.clone())),
            key_expr("model", Expr::String(card.model.clone())),
            key_expr("available", Expr::Bool(false)),
            key_expr("accepted", Expr::Bool(false)),
            key_expr("score", float_expr(f64::MAX)),
            key_expr("health", health_status(card)),
            key_expr("privacy", Expr::Symbol(Symbol::new("rejected"))),
            key_expr("reason", Expr::String(reason)),
        ]));
    }

    pub(super) fn bid(&mut self, card: &ModelCard, bid: &ModelBid, score: f64, accepted: bool) {
        let mut entries = vec![
            key_expr("runner", Expr::Symbol(card.runner.clone())),
            key_expr("model", Expr::String(card.model.clone())),
            key_expr("available", Expr::Bool(bid.available)),
            key_expr("accepted", Expr::Bool(accepted)),
            key_expr("score", float_expr(score)),
            key_expr("health", health_status(card)),
            key_expr("privacy", Expr::Symbol(Symbol::new("accepted"))),
        ];
        if let Some(cost) = card_cost(card) {
            entries.push(key_expr("cost-usd", float_expr(cost)));
        }
        if let Some(reason) = &bid.reason {
            entries.push(key_expr("reason", Expr::String(reason.clone())));
        }
        self.bids.push(Expr::Map(entries));
    }

    pub(super) fn candidate(&mut self, candidate: &MarketCandidate) {
        let mut entries = vec![
            key_expr("runner", Expr::Symbol(candidate.card.runner.clone())),
            key_expr("model", Expr::String(candidate.card.model.clone())),
            key_expr("score", float_expr(candidate.score)),
            key_expr("latency-ms", number_expr(candidate.latency_ms())),
        ];
        if let Some(model) = &candidate.bid.model {
            entries.push(key_expr("bid-model", Expr::String(model.clone())));
        }
        self.candidates.push(Expr::Map(entries));
    }

    pub(super) fn run(&mut self, runner: Symbol, phase: &str) {
        self.runs.push(Expr::Map(vec![
            key_expr("runner", Expr::Symbol(runner)),
            key_expr("phase", Expr::Symbol(Symbol::new(phase))),
            key_expr("status", Expr::Symbol(Symbol::new("started"))),
        ]));
    }

    pub(super) fn response(&mut self, runner: Symbol, phase: &str, response: &ModelResponse) {
        self.runs.push(Expr::Map(vec![
            key_expr("runner", Expr::Symbol(runner)),
            key_expr("phase", Expr::Symbol(Symbol::new(phase))),
            key_expr("status", Expr::Symbol(Symbol::new("response"))),
            key_expr("response", Expr::from(response.clone())),
        ]));
    }

    pub(super) fn cancelled(&mut self, runner: Symbol, reason: &str) {
        self.cancellations.push(Expr::Map(vec![
            key_expr("runner", Expr::Symbol(runner)),
            key_expr("reason", Expr::String(reason.to_owned())),
        ]));
    }

    pub(super) fn failure(&mut self, runner: Symbol, message: String) {
        self.failures.push(Expr::Map(vec![
            key_expr("runner", Expr::Symbol(runner)),
            key_expr("message", Expr::String(message)),
        ]));
    }

    pub(super) fn to_expr(&self) -> Expr {
        let selected = self
            .selected
            .as_ref()
            .map(|symbol| Expr::Symbol(symbol.clone()))
            .unwrap_or_else(|| self.selected_from_bids());
        let mut entries = vec![
            key_bool("market-decision", true),
            key_expr(
                "execution",
                Expr::Symbol(
                    self.policy
                        .execution
                        .clone()
                        .unwrap_or_else(|| Symbol::new("select")),
                ),
            ),
            key_expr("policy", self.policy.to_expr()),
            key_expr("selected", selected),
            key_expr("bids", Expr::List(self.bids.clone())),
            key_expr("candidates", Expr::List(self.candidates.clone())),
            key_expr("runs", Expr::List(self.runs.clone())),
            key_expr("cancellations", Expr::List(self.cancellations.clone())),
            key_bool("fallback-used", self.fallback_used.is_some()),
            key_expr("failures", Expr::List(self.failures.clone())),
        ];
        if !self.privacy.is_empty() {
            entries.push(key_expr("privacy-policy", self.privacy.to_expr()));
        }
        if let Some(response) = &self.selected_response {
            entries.push(key_expr("selected-response", response.clone()));
        }
        if let Some(fallback) = &self.fallback_used {
            entries.push(key_expr("fallback", Expr::Symbol(fallback.clone())));
        }
        if let Some(verifier) = &self.verified_by {
            entries.push(key_expr("verified-by", Expr::Symbol(verifier.clone())));
        }
        Expr::Map(entries)
    }

    fn selected_from_bids(&self) -> Expr {
        self.bids
            .iter()
            .filter_map(|bid| {
                if matches!(field(bid, "accepted"), Some(Expr::Bool(true))) {
                    Some((
                        field(bid, "score").and_then(expr_f64).unwrap_or(f64::MAX),
                        field(bid, "runner").cloned().unwrap_or(Expr::Nil),
                    ))
                } else {
                    None
                }
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, runner)| runner)
            .unwrap_or(Expr::Nil)
    }
}

pub(super) fn request_allows_branching(request: &ModelRequest) -> bool {
    if request_bool_field(request, "idempotent").unwrap_or(false) {
        return true;
    }
    !["tool-calls", "tool-results", "tool-continuation"]
        .iter()
        .any(|field| request_has_field(request, field))
}

pub(super) fn response_should_escalate(response: &ModelResponse) -> bool {
    is_error_response(response)
        || super::market_cards::is_retryable_error(response)
        || shape_failed(response)
        || low_confidence(response)
}

pub(super) fn is_error_response(response: &ModelResponse) -> bool {
    response.stop_reason.name.as_ref() == "error"
}

pub(super) fn response_failure_reason(response: &ModelResponse) -> Option<String> {
    if is_error_response(response) {
        return Some("model response stop-reason was error".to_owned());
    }
    if shape_failed(response) {
        return Some("model response failed output-shape".to_owned());
    }
    if low_confidence(response) {
        return Some("model response confidence below threshold".to_owned());
    }
    None
}

pub(super) fn verifier_accepts(response: &ModelResponse) -> bool {
    if response_should_escalate(response) {
        return false;
    }
    let text = response_text(response).to_ascii_lowercase();
    if ["false", "reject", "fail", "mismatch", "no"]
        .iter()
        .any(|needle| text.contains(needle))
    {
        return false;
    }
    if ["true", "accept", "ok", "verified", "pass"]
        .iter()
        .any(|needle| text.contains(needle))
    {
        return true;
    }
    true
}

fn request_bool_field(request: &ModelRequest, name: &str) -> Option<bool> {
    request_field(request, name).and_then(|expr| match expr {
        Expr::Bool(value) => Some(*value),
        _ => None,
    })
}

fn request_has_field(request: &ModelRequest, name: &str) -> bool {
    request_field(request, name).is_some()
}

fn request_field<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a Expr> {
    request.extra.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}

fn shape_failed(response: &ModelResponse) -> bool {
    matches!(
        response_extra(response, "shape-ok"),
        Some(Expr::Bool(false))
    )
}

fn low_confidence(response: &ModelResponse) -> bool {
    response_extra(response, "confidence")
        .and_then(expr_f64)
        .is_some_and(|confidence| confidence < 0.5)
}

fn response_text(response: &ModelResponse) -> String {
    response
        .content
        .iter()
        .map(expr_text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn response_extra<'a>(response: &'a ModelResponse, name: &str) -> Option<&'a Expr> {
    response.extra.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
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

fn number_expr(value: u64) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => format!("{expr:?}"),
    }
}
