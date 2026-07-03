use crate::{AgentComponent, RunnerBackend};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrivacyPolicy {
    pub(crate) local_only: bool,
    pub(crate) metadata_only: bool,
    pub(crate) no_raw: bool,
    pub(crate) allow_tools: Option<BTreeSet<Symbol>>,
}

impl PrivacyPolicy {
    pub(crate) fn from_request_expr(expr: &Expr) -> Result<Self> {
        let mut policy = field(expr, "privacy")
            .map(Self::from_policy_expr)
            .transpose()?
            .unwrap_or_default();
        if let Some(allow_tools) = field(expr, "allow-tools") {
            policy.allow_tools = Some(symbol_set(allow_tools, "allow-tools")?);
        }
        Ok(policy)
    }

    pub(crate) fn from_model_request(request: &ModelRequest) -> Result<Self> {
        Self::from_request_expr(&request.clone().into())
    }

    pub(crate) fn from_payload(payload: &Expr) -> Result<Self> {
        if is_model_request(payload) {
            return Self::from_request_expr(payload);
        }
        if let Some(expr) = field(payload, "expr")
            && is_model_request(expr)
        {
            return Self::from_request_expr(expr);
        }
        Ok(Self::default())
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.local_only && !self.metadata_only && !self.no_raw && self.allow_tools.is_none()
    }

    pub(crate) fn runner_denial(
        &self,
        runner: &Symbol,
        locality: &Symbol,
        remote_like_address: bool,
    ) -> Option<String> {
        if !self.local_only {
            return None;
        }
        if remote_like_address {
            return Some(format!(
                "privacy local-only rejected runner {runner}: remote-like address"
            ));
        }
        if !locality_is_allowed_for_local_only(locality) {
            return Some(format!(
                "privacy local-only rejected runner {runner}: locality {locality}"
            ));
        }
        None
    }

    pub(crate) fn card_denial(&self, card: &ModelCard) -> Option<String> {
        self.runner_denial(&card.runner, &card.locality, false)
    }

    pub(crate) fn allows_tool_content(&self, tool: &Symbol) -> bool {
        self.allow_tools
            .as_ref()
            .is_none_or(|allowed| allowed.contains(tool))
    }

    pub(crate) fn ensure_no_raw_refs(&self, expr: &Expr) -> Result<()> {
        if self.no_raw && contains_raw_ref(expr) {
            return Err(Error::Eval(
                "privacy no-raw rejected raw-ref in model request".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn to_expr(&self) -> Expr {
        let mut entries = Vec::new();
        if self.local_only {
            entries.push(key_bool("local-only", true));
        }
        if self.metadata_only {
            entries.push(key_bool("metadata-only", true));
        }
        if self.no_raw {
            entries.push(key_bool("no-raw", true));
        }
        if let Some(tools) = &self.allow_tools {
            entries.push(key_expr(
                "allow-tools",
                Expr::List(tools.iter().cloned().map(Expr::Symbol).collect()),
            ));
        }
        Expr::Map(entries)
    }

    fn from_policy_expr(expr: &Expr) -> Result<Self> {
        let mut policy = Self::default();
        policy.apply_policy_expr(expr)?;
        Ok(policy)
    }

    fn apply_policy_expr(&mut self, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Nil => Ok(()),
            Expr::Symbol(symbol) => self.apply_policy_atom(symbol),
            Expr::String(text) => self.apply_policy_atom(&symbol_from_text(text)),
            Expr::List(items) | Expr::Vector(items) | Expr::Set(items) => {
                for item in items {
                    self.apply_policy_expr(item)?;
                }
                Ok(())
            }
            Expr::Map(entries) => {
                for (key, value) in entries {
                    let Some(name) = field_name(key) else {
                        continue;
                    };
                    match name {
                        "local-only" => self.local_only = truthy(value),
                        "metadata-only" => self.metadata_only = truthy(value),
                        "no-raw" => self.no_raw = truthy(value),
                        "allow-tools" => {
                            self.allow_tools = Some(symbol_set(value, "privacy allow-tools")?);
                        }
                        "policies" | "policy" => self.apply_policy_expr(value)?,
                        _ => {}
                    }
                }
                Ok(())
            }
            _ => Err(Error::Eval(
                "privacy policy expects symbols, strings, lists, or maps".to_owned(),
            )),
        }
    }

    fn apply_policy_atom(&mut self, symbol: &Symbol) -> Result<()> {
        match symbol.name.as_ref() {
            "local-only" => self.local_only = true,
            "metadata-only" => self.metadata_only = true,
            "no-raw" => self.no_raw = true,
            other => {
                return Err(Error::Eval(format!("unknown privacy policy {other}")));
            }
        }
        Ok(())
    }
}

pub(crate) fn enforce_component_runner_policy(
    component: &AgentComponent,
    backend: &RunnerBackend,
    expr: &Expr,
) -> Result<()> {
    let policy = PrivacyPolicy::from_request_expr(expr)?;
    policy.ensure_no_raw_refs(expr)?;
    if let Some(message) = policy.runner_denial(
        &component.symbol,
        &runner_locality(backend),
        component.address.is_remote_like(),
    ) {
        return Err(Error::Eval(message));
    }
    Ok(())
}

pub(crate) fn redact_trace_entry(entry: Expr) -> Result<Expr> {
    let Expr::Map(mut entries) = entry else {
        return Ok(entry);
    };
    let task_id = entry_field(&entries, "task-id").and_then(expr_string);
    let Some(payload_index) = entries.iter().position(|(key, _)| is_field(key, "payload")) else {
        return Ok(Expr::Map(entries));
    };
    let payload = entries[payload_index].1.clone();
    let payload_policy = PrivacyPolicy::from_payload(&payload)?;
    if let Some(task_id) = &task_id
        && !payload_policy.is_empty()
    {
        remember_task_policy(task_id, payload_policy.clone())?;
    }
    let policy = task_id
        .as_deref()
        .and_then(task_policy)
        .unwrap_or(payload_policy);
    if policy.metadata_only {
        entries[payload_index].1 = metadata_payload(&payload);
        if !entries.iter().any(|(key, _)| is_field(key, "privacy")) {
            entries.push(key_expr("privacy", policy.to_expr()));
        }
    }
    Ok(Expr::Map(entries))
}

fn runner_locality(backend: &RunnerBackend) -> Symbol {
    match backend {
        RunnerBackend::Echo { .. }
        | RunnerBackend::Cassette { .. }
        | RunnerBackend::Fake { .. } => Symbol::new("local"),
        RunnerBackend::External { runner } => runner.card().locality,
    }
}

fn remember_task_policy(task_id: &str, policy: PrivacyPolicy) -> Result<()> {
    let mut policies = task_policies()
        .lock()
        .map_err(|_| Error::PoisonedLock("privacy policy registry"))?;
    policies.insert(task_id.to_owned(), policy);
    Ok(())
}

fn task_policy(task_id: &str) -> Option<PrivacyPolicy> {
    task_policies().lock().ok()?.get(task_id).cloned()
}

fn task_policies() -> &'static Mutex<BTreeMap<String, PrivacyPolicy>> {
    static POLICIES: OnceLock<Mutex<BTreeMap<String, PrivacyPolicy>>> = OnceLock::new();
    POLICIES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn metadata_payload(payload: &Expr) -> Expr {
    let mut entries = vec![
        key_bool("privacy-redacted", true),
        key_expr("payload-hash", Expr::String(hash_expr(payload))),
    ];
    if let Some(expr) = field(payload, "expr") {
        entries.push(key_expr("expr-hash", Expr::String(hash_expr(expr))));
        entries.extend(model_metadata(expr));
    } else if let Some(value) = field(payload, "value") {
        entries.push(key_expr("value-hash", Expr::String(hash_expr(value))));
        entries.extend(model_metadata(value));
    } else {
        entries.extend(model_metadata(payload));
    }
    Expr::Map(entries)
}

fn model_metadata(expr: &Expr) -> Vec<(Expr, Expr)> {
    let mut entries = Vec::new();
    if is_model_request(expr) {
        entries.push(key_expr(
            "transcript",
            Expr::Symbol(Symbol::new("model-request")),
        ));
        if let Some(task) = field(expr, "task") {
            entries.push(key_expr("task-hash", Expr::String(hash_expr(task))));
        }
        if let Some(messages) = field(expr, "messages") {
            entries.push(key_expr("messages-hash", Expr::String(hash_expr(messages))));
        }
        if let Some(tools) = field(expr, "tools") {
            entries.push(key_expr("tools-hash", Expr::String(hash_expr(tools))));
        }
    } else if marker_is_true(expr, "model-response") {
        entries.push(key_expr(
            "transcript",
            Expr::Symbol(Symbol::new("model-response")),
        ));
        copy_field(expr, "runner", &mut entries);
        copy_field(expr, "model", &mut entries);
        copy_field(expr, "stop-reason", &mut entries);
        copy_field(expr, "usage", &mut entries);
        copy_field(expr, "market-decision", &mut entries);
        copy_field(expr, "cache-hit", &mut entries);
        if let Some(content) = field(expr, "content") {
            entries.push(key_expr("content-hash", Expr::String(hash_expr(content))));
        }
    } else if marker_is_true(expr, "model-event") {
        entries.push(key_expr(
            "transcript",
            Expr::Symbol(Symbol::new("model-event")),
        ));
        copy_field(expr, "event", &mut entries);
        copy_field(expr, "runner", &mut entries);
        copy_field(expr, "model", &mut entries);
        copy_field(expr, "span-id", &mut entries);
        if let Some(response) = field(expr, "response") {
            entries.push(key_expr("response-hash", Expr::String(hash_expr(response))));
        }
    } else if marker_is_true(expr, "model-card") {
        entries.push(key_expr(
            "transcript",
            Expr::Symbol(Symbol::new("model-card")),
        ));
        copy_field(expr, "runner", &mut entries);
        copy_field(expr, "model", &mut entries);
        copy_field(expr, "provider", &mut entries);
        copy_field(expr, "locality", &mut entries);
    }
    entries
}

fn copy_field(expr: &Expr, name: &str, entries: &mut Vec<(Expr, Expr)>) {
    if let Some(value) = field(expr, name) {
        entries.push(key_expr(name, value.clone()));
    }
}

fn contains_raw_ref(expr: &Expr) -> bool {
    match expr {
        Expr::Symbol(symbol) => symbol.name.as_ref() == "raw-ref",
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().any(contains_raw_ref)
        }
        Expr::Map(entries) => entries
            .iter()
            .any(|(key, value)| is_field(key, "raw-ref") || contains_raw_ref(value)),
        Expr::Call { operator, args } => {
            contains_raw_ref(operator) || args.iter().any(contains_raw_ref)
        }
        Expr::Infix { left, right, .. } => contains_raw_ref(left) || contains_raw_ref(right),
        Expr::Prefix { arg, .. } | Expr::Postfix { arg, .. } => contains_raw_ref(arg),
        Expr::Quote { expr, .. } | Expr::Annotated { expr, .. } => contains_raw_ref(expr),
        Expr::Extension { payload, .. } => contains_raw_ref(payload),
        _ => false,
    }
}

fn locality_is_allowed_for_local_only(locality: &Symbol) -> bool {
    matches!(
        locality.name.as_ref(),
        "local" | "agent" | "agent-backed" | "fabric" | "in-process" | "process"
    )
}

fn symbol_set(expr: &Expr, label: &str) -> Result<BTreeSet<Symbol>> {
    let items = match expr {
        Expr::Nil => Vec::new(),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) => items.clone(),
        other => vec![other.clone()],
    };
    items
        .iter()
        .map(|item| symbol_from_expr(item, label))
        .collect()
}

fn symbol_from_expr(expr: &Expr, label: &str) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        Expr::String(text) => Ok(symbol_from_text(text)),
        _ => Err(Error::Eval(format!("{label} expects symbols or strings"))),
    }
}

fn symbol_from_text(text: &str) -> Symbol {
    match text.split_once('/') {
        Some((namespace, name)) => Symbol::qualified(namespace.to_owned(), name.to_owned()),
        None => Symbol::new(text.to_owned()),
    }
}

fn truthy(expr: &Expr) -> bool {
    !matches!(expr, Expr::Bool(false) | Expr::Nil)
}

fn is_model_request(expr: &Expr) -> bool {
    marker_is_true(expr, "model-request")
}

fn marker_is_true(expr: &Expr, name: &str) -> bool {
    matches!(field(expr, name), Some(Expr::Bool(true)))
}

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entry_field(entries, name)
}

fn entry_field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    entries.iter().find_map(|(key, value)| {
        if is_field(key, name) {
            Some(value)
        } else {
            None
        }
    })
}

fn is_field(expr: &Expr, name: &str) -> bool {
    matches!(field_name(expr), Some(found) if found == name)
}

fn field_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.as_ref()),
        _ => None,
    }
}

fn expr_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn hash_expr(expr: &Expr) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{expr:?}").hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

use sim_value::build::entry as key_expr;
