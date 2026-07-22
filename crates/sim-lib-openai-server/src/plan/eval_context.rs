use sim_kernel::Expr;

use crate::{
    plan::eval_helpers::{keyword_value, request_field, request_field_text},
    runtime::OpenAiFederationPolicy,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanPrivacy {
    LocalOnly,
    AllowRemote,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EvalContext {
    pub(crate) privacy: PlanPrivacy,
    privacy_label: Option<String>,
    budget: Vec<(String, Expr)>,
    trace: bool,
}

impl EvalContext {
    pub(crate) fn from_request(request: &Expr) -> Self {
        let privacy_label = request_field_text(request, "privacy");
        Self {
            privacy: if privacy_label.as_deref() == Some("local-only") {
                PlanPrivacy::LocalOnly
            } else {
                PlanPrivacy::AllowRemote
            },
            privacy_label,
            budget: request_budget_entries(request),
            trace: false,
        }
    }

    pub(crate) fn local(&self) -> Self {
        Self {
            privacy: PlanPrivacy::LocalOnly,
            privacy_label: Some("local-only".to_owned()),
            budget: self.budget.clone(),
            trace: self.trace,
        }
    }

    pub(crate) fn trace(&self) -> Self {
        Self {
            trace: true,
            privacy: self.privacy,
            privacy_label: self.privacy_label.clone(),
            budget: self.budget.clone(),
        }
    }

    pub(crate) fn budgeted(&self, args: &[Expr]) -> Self {
        Self {
            privacy: self.privacy,
            privacy_label: self.privacy_label.clone(),
            budget: self
                .budget
                .iter()
                .cloned()
                .chain(budget_keyword_entries(args))
                .collect(),
            trace: self.trace,
        }
    }

    pub(crate) fn federation_policy(&self) -> OpenAiFederationPolicy {
        OpenAiFederationPolicy::new(self.privacy_label.clone(), self.budget.clone())
    }
}

fn request_budget_entries(request: &Expr) -> Vec<(String, Expr)> {
    let mut entries = Vec::new();
    if let Some(Expr::Map(fields)) = request_field(request, "budget") {
        entries.extend(
            fields.iter().filter_map(|(key, value)| {
                key_name(key).map(|name| (name.to_owned(), value.clone()))
            }),
        );
    }
    for name in [
        "max-tokens",
        "max-output-tokens",
        "max-cost",
        "max-latency-ms",
    ] {
        if let Some(value) = request_field(request, name) {
            entries.push((name.to_owned(), value.clone()));
        }
    }
    entries
}

fn budget_keyword_entries(args: &[Expr]) -> Vec<(String, Expr)> {
    ["max-tokens", "max-cost", "max-latency-ms"]
        .into_iter()
        .filter_map(|name| keyword_value(args, name).map(|value| (name.to_owned(), value.clone())))
        .collect()
}

fn key_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.as_ref()),
        Expr::String(value) => Some(value.as_str()),
        _ => None,
    }
}
