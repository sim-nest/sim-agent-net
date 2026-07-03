use crate::{
    AI_RUNNER_CACHE_CAPABILITY,
    memory::{append_memory_log, load_memory_log},
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{CapabilityName, Cx, Error, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelRequest};
use sim_lib_server::parse_duration;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct StoredCacheEntry {
    stored_at_ms: u64,
    response: Expr,
}

#[derive(Clone)]
struct CachePolicy {
    read: bool,
    write: bool,
    path: Option<PathBuf>,
    ttl: Option<Duration>,
    semantic_key: Option<Expr>,
    idempotent: bool,
}

pub(crate) struct RunnerCachePlan {
    policy: Option<CachePolicy>,
    key: Option<String>,
    hit: Option<Expr>,
}

pub(crate) struct CacheEventSink<'a> {
    inner: &'a mut dyn ModelEventSink,
}

impl<'a> CacheEventSink<'a> {
    pub(crate) fn new(inner: &'a mut dyn ModelEventSink) -> Self {
        Self { inner }
    }
}

impl ModelEventSink for CacheEventSink<'_> {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        if event.event == Symbol::new("final") {
            return Ok(());
        }
        self.inner.emit(event)
    }
}

impl RunnerCachePlan {
    pub(crate) fn prepare(
        cx: &mut Cx,
        request: &EvalRequest,
        runner: &Symbol,
        model: &str,
    ) -> Result<Self> {
        let Some(policy) = CachePolicy::from_request(&request.expr)? else {
            return Ok(Self::inactive());
        };
        if (!policy.read && !policy.write)
            || (has_tool_continuation(&request.expr) && !policy.idempotent)
        {
            return Ok(Self::inactive());
        }
        let key = stable_cache_key(cx, request, runner, model, policy.semantic_key.as_ref())?;
        let hit = if policy.read {
            lookup_cache_entry(&policy, &key)?
                .map(|entry| set_cache_hit(entry.response, true))
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            policy: Some(policy),
            key: Some(key),
            hit,
        })
    }

    pub(crate) fn hit(&self) -> Option<Expr> {
        self.hit.clone()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.policy.is_some()
    }

    pub(crate) fn require_write_capability(&self, cx: &mut Cx) -> Result<()> {
        if self
            .policy
            .as_ref()
            .is_some_and(|policy| policy.write && policy.path.is_some())
        {
            cx.require(&CapabilityName::new(AI_RUNNER_CACHE_CAPABILITY))?;
        }
        Ok(())
    }

    pub(crate) fn finish(&self, response: Expr) -> Result<Expr> {
        let Some(policy) = &self.policy else {
            return Ok(response);
        };
        let response = set_cache_hit(response, false)?;
        if policy.write {
            let key = self.key.as_ref().ok_or_else(|| {
                Error::Eval("model cache plan missing stable cache key".to_owned())
            })?;
            write_cache_entry(policy, key, response.clone())?;
        }
        Ok(response)
    }

    fn inactive() -> Self {
        Self {
            policy: None,
            key: None,
            hit: None,
        }
    }
}

impl CachePolicy {
    fn from_request(expr: &Expr) -> Result<Option<Self>> {
        let Some(cache) = field(expr, "cache") else {
            return Ok(None);
        };
        if matches!(cache, Expr::Nil | Expr::Bool(false)) {
            return Ok(None);
        }
        let (read, write, path, ttl, semantic_key, cache_idempotent) = match cache {
            Expr::Bool(true) => (true, true, None, None, None, false),
            Expr::Map(_) => (
                cache_reads(cache)?,
                cache_writes(cache)?,
                cache_path(cache)?,
                field(cache, "ttl").map(parse_duration).transpose()?,
                field(cache, "semantic-key").cloned(),
                bool_field(cache, "idempotent"),
            ),
            _ => {
                return Err(Error::Eval(
                    "model request cache policy must be a map or boolean".to_owned(),
                ));
            }
        };
        Ok(Some(Self {
            read,
            write,
            path,
            ttl,
            semantic_key,
            idempotent: bool_field(expr, "idempotent") || cache_idempotent,
        }))
    }
}

fn cache_reads(cache: &Expr) -> Result<bool> {
    let mode = cache_mode(cache)?;
    Ok(!matches!(
        mode.as_deref(),
        Some("off" | "none" | "disabled" | "write" | "write-only" | "refresh")
    ))
}

fn cache_writes(cache: &Expr) -> Result<bool> {
    let mode = cache_mode(cache)?;
    Ok(!matches!(
        mode.as_deref(),
        Some("off" | "none" | "disabled" | "read" | "read-only")
    ))
}

fn cache_mode(cache: &Expr) -> Result<Option<String>> {
    field(cache, "mode").map(cache_mode_name).transpose()
}

fn cache_mode_name(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Ok(symbol.name.to_string()),
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(
            "model cache mode must be a symbol or string".to_owned(),
        )),
    }
}

fn cache_path(cache: &Expr) -> Result<Option<PathBuf>> {
    let value = field(cache, "path")
        .or_else(|| field(cache, "file"))
        .or_else(|| field(cache, "journal"));
    value
        .map(|expr| match expr {
            Expr::String(text) => Ok(PathBuf::from(text)),
            _ => Err(Error::Eval("model cache path must be a string".to_owned())),
        })
        .transpose()
}

pub(super) fn stable_cache_key(
    cx: &mut Cx,
    request: &EvalRequest,
    runner: &Symbol,
    model: &str,
    semantic_key: Option<&Expr>,
) -> Result<String> {
    let model_request = ModelRequest::try_from(request.expr.clone())?;
    let shape = request
        .result_shape
        .as_ref()
        .map(|shape| shape.object().as_expr(cx))
        .transpose()?
        .unwrap_or(Expr::Nil);
    let mut capabilities = request
        .required_capabilities
        .iter()
        .map(|capability| Expr::String(format!("{capability:?}")))
        .collect::<Vec<_>>();
    capabilities.sort_by_key(|expr| format!("{expr:?}"));
    let mut entries = vec![
        key_expr("runner", Expr::Symbol(runner.clone())),
        key_expr("model", Expr::String(model.to_owned())),
        key_expr("request", request_without_cache(Expr::from(model_request))),
        key_expr("result-shape", shape),
        key_expr("required-capabilities", Expr::List(capabilities)),
    ];
    if let Some(key) = semantic_key {
        entries.push(key_expr("semantic-key", key.clone()));
    }
    Ok(format!("{:?}", normalize_expr(&Expr::Map(entries))))
}

fn request_without_cache(expr: Expr) -> Expr {
    let Expr::Map(entries) = expr else {
        return expr;
    };
    Expr::Map(
        entries
            .into_iter()
            .filter(|(key, _)| !symbol_key_is(key, "cache"))
            .collect(),
    )
}

fn lookup_cache_entry(policy: &CachePolicy, key: &str) -> Result<Option<StoredCacheEntry>> {
    match &policy.path {
        Some(path) => persistent_lookup(path, policy, key),
        None => memory_lookup(policy, key),
    }
}

fn memory_lookup(policy: &CachePolicy, key: &str) -> Result<Option<StoredCacheEntry>> {
    let mut cache = memory_cache()
        .lock()
        .map_err(|_| Error::PoisonedLock("model cache"))?;
    let Some(entry) = cache.get(key).cloned() else {
        return Ok(None);
    };
    if entry_is_stale(policy, entry.stored_at_ms) {
        cache.remove(key);
        return Ok(None);
    }
    Ok(Some(entry))
}

fn persistent_lookup(
    path: &Path,
    policy: &CachePolicy,
    key: &str,
) -> Result<Option<StoredCacheEntry>> {
    for entry in load_memory_log(path)?.iter().rev() {
        let Some(stored) = decode_cache_entry(entry, key)? else {
            continue;
        };
        if entry_is_stale(policy, stored.stored_at_ms) {
            return Ok(None);
        }
        return Ok(Some(stored));
    }
    Ok(None)
}

fn write_cache_entry(policy: &CachePolicy, key: &str, response: Expr) -> Result<()> {
    let stored_at_ms = now_ms();
    match &policy.path {
        Some(path) => append_memory_log(path, &cache_entry_expr(key, stored_at_ms, response)),
        None => {
            memory_cache()
                .lock()
                .map_err(|_| Error::PoisonedLock("model cache"))?
                .insert(
                    key.to_owned(),
                    StoredCacheEntry {
                        stored_at_ms,
                        response,
                    },
                );
            Ok(())
        }
    }
}

fn decode_cache_entry(expr: &Expr, wanted_key: &str) -> Result<Option<StoredCacheEntry>> {
    if !matches!(field(expr, "model-cache-entry"), Some(Expr::Bool(true))) {
        return Ok(None);
    }
    if !matches!(field(expr, "key"), Some(Expr::String(key)) if key == wanted_key) {
        return Ok(None);
    }
    let stored_at_ms = field(expr, "stored-at-ms")
        .ok_or_else(|| Error::Eval("model cache entry missing stored-at-ms".to_owned()))
        .and_then(parse_u64)?;
    let response = field(expr, "response")
        .ok_or_else(|| Error::Eval("model cache entry missing response".to_owned()))?
        .clone();
    validate_chat_transcript(&response)?;
    Ok(Some(StoredCacheEntry {
        stored_at_ms,
        response,
    }))
}

fn cache_entry_expr(key: &str, stored_at_ms: u64, response: Expr) -> Expr {
    Expr::Map(vec![
        key_bool("model-cache-entry", true),
        key_expr("key", Expr::String(key.to_owned())),
        key_expr("stored-at-ms", Expr::String(stored_at_ms.to_string())),
        key_expr("response", response),
    ])
}

fn entry_is_stale(policy: &CachePolicy, stored_at_ms: u64) -> bool {
    let Some(ttl) = policy.ttl else {
        return false;
    };
    let ttl_ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
    now_ms() >= stored_at_ms.saturating_add(ttl_ms)
}

fn has_tool_continuation(expr: &Expr) -> bool {
    match expr {
        Expr::Map(entries) => {
            if bool_field(expr, "tool-continuation") {
                return true;
            }
            if matches!(
                field(expr, "role"),
                Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "tool"
            ) || matches!(field(expr, "role"), Some(Expr::String(role)) if role == "tool")
            {
                return true;
            }
            entries.iter().any(|(key, value)| {
                tool_continuation_key(key)
                    || has_tool_continuation(key)
                    || has_tool_continuation(value)
            })
        }
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().any(has_tool_continuation)
        }
        Expr::Call { operator, args } => {
            has_tool_continuation(operator) || args.iter().any(has_tool_continuation)
        }
        Expr::Quote { expr, .. } => has_tool_continuation(expr),
        Expr::Annotated { expr, annotations } => {
            has_tool_continuation(expr)
                || annotations
                    .iter()
                    .any(|(_, value)| has_tool_continuation(value))
        }
        _ => false,
    }
}

fn tool_continuation_key(expr: &Expr) -> bool {
    let Expr::Symbol(symbol) = expr else {
        return false;
    };
    matches!(
        symbol.name.as_ref(),
        "tool-result"
            | "tool-results"
            | "tool-response"
            | "tool-responses"
            | "tool-call-id"
            | "tool-call-result"
            | "tool-call-results"
    )
}

pub(crate) fn normalize_expr(expr: &Expr) -> Expr {
    match expr {
        Expr::List(items) => Expr::List(items.iter().map(normalize_expr).collect()),
        Expr::Vector(items) => Expr::Vector(items.iter().map(normalize_expr).collect()),
        Expr::Set(items) => Expr::Set(items.iter().map(normalize_expr).collect()),
        Expr::Block(items) => Expr::Block(items.iter().map(normalize_expr).collect()),
        Expr::Map(entries) => {
            let mut normalized = entries
                .iter()
                .map(|(key, value)| (normalize_expr(key), normalize_expr(value)))
                .collect::<Vec<_>>();
            normalized.sort_by_key(|(key, _)| format!("{key:?}"));
            Expr::Map(normalized)
        }
        Expr::Call { operator, args } => Expr::Call {
            operator: Box::new(normalize_expr(operator)),
            args: args.iter().map(normalize_expr).collect(),
        },
        Expr::Quote { mode, expr } => Expr::Quote {
            mode: *mode,
            expr: Box::new(normalize_expr(expr)),
        },
        Expr::Annotated { expr, annotations } => Expr::Annotated {
            expr: Box::new(normalize_expr(expr)),
            annotations: annotations
                .iter()
                .map(|(key, value)| (key.clone(), normalize_expr(value)))
                .collect(),
        },
        _ => expr.clone(),
    }
}

pub(crate) fn set_cache_hit(expr: Expr, hit: bool) -> Result<Expr> {
    let Expr::Map(mut entries) = expr else {
        return Err(Error::Eval(
            "runner response must be a model-response map".to_owned(),
        ));
    };
    entries.retain(|(key, _)| !symbol_key_is(key, "cache-hit"));
    entries.push((Expr::Symbol(Symbol::new("cache-hit")), Expr::Bool(hit)));
    let out = Expr::Map(entries);
    validate_chat_transcript(&out)?;
    Ok(out)
}

fn memory_cache() -> &'static Mutex<HashMap<String, StoredCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, StoredCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

use sim_value::access::field;

fn bool_field(expr: &Expr, name: &str) -> bool {
    matches!(field(expr, name), Some(Expr::Bool(true)))
}

fn symbol_key_is(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name)
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

use sim_value::build::entry as key_expr;

fn parse_u64(expr: &Expr) -> Result<u64> {
    match expr {
        Expr::String(text) => text
            .parse::<u64>()
            .map_err(|_| Error::Eval("model cache timestamp must be an integer".to_owned())),
        Expr::Number(number) => number
            .canonical
            .parse::<u64>()
            .map_err(|_| Error::Eval("model cache timestamp must be an integer".to_owned())),
        _ => Err(Error::Eval(
            "model cache timestamp must be a string or number".to_owned(),
        )),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
