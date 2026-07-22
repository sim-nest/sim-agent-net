use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};
use sim_kernel::{CapabilityName, ContentId, Expr, NumberLiteral, Symbol};
use sim_lib_net_core::hex_encode;
use sim_value::access::field_any;

use crate::{
    capabilities::OPENAI_GATEWAY_ADMIN_CAPABILITY,
    objects::{GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun, content_id_expr},
    server::GatewayRouteState,
    storage::GatewayStoreCounts,
};

use super::errors::OpenAiRouteError;

/// Route path listing all recorded gateway runs.
pub const ADMIN_RUNS_PATH: &str = "/v1/sim/admin/runs";
/// Route template for retrieving a single run by id.
pub const ADMIN_RUN_RETRIEVAL_ROUTE: &str = "/v1/sim/admin/runs/{id}";
/// Path prefix stripped to extract a run id from a run-retrieval request.
pub const ADMIN_RUN_RETRIEVAL_PREFIX: &str = "/v1/sim/admin/runs/";
/// Route path listing all recorded gateway events.
pub const ADMIN_EVENTS_PATH: &str = "/v1/sim/admin/events";
/// Route path reporting storage object counts and latency stats.
pub const ADMIN_STORAGE_STATS_PATH: &str = "/v1/sim/admin/storage-stats";
/// Route path reporting backend and registered-model health.
pub const ADMIN_MODEL_HEALTH_PATH: &str = "/v1/sim/admin/model-health";
/// Route path reporting plan-cache statistics.
pub const ADMIN_CACHE_STATS_PATH: &str = "/v1/sim/admin/cache-stats";
/// Route path reporting key and capability configuration.
pub const ADMIN_CAPABILITY_REPORT_PATH: &str = "/v1/sim/admin/capability-report";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AdminCounters {
    request_count: usize,
    run_count: usize,
    event_count: usize,
    error_count: usize,
    stream_count: usize,
    active_streams: usize,
}

impl AdminCounters {
    fn from_ledger(
        counts: GatewayStoreCounts,
        requests: &[(ContentId, GatewayRequest)],
        events: &[(ContentId, GatewayEvent)],
    ) -> Self {
        Self {
            request_count: counts.request_count,
            run_count: counts.run_count,
            event_count: counts.event_count,
            error_count: events
                .iter()
                .filter(|(_, event)| event.kind().name.as_ref() == "error")
                .count(),
            stream_count: requests
                .iter()
                .filter(|(_, request)| request_streams(request))
                .count(),
            active_streams: 0,
        }
    }

    fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("request-count", usize_expr(self.request_count)),
            field("run-count", usize_expr(self.run_count)),
            field("event-count", usize_expr(self.event_count)),
            field("error-count", usize_expr(self.error_count)),
            field("stream-count", usize_expr(self.stream_count)),
            field("active-streams", usize_expr(self.active_streams)),
        ])
    }
}

/// Handles the admin runs listing, returning every recorded run as JSON.
pub fn handle_admin_runs(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    admin_json_response(request, state, || runs_expr(state))
}

/// Handles admin retrieval of a single run by the id embedded in the path.
pub fn handle_admin_run_get(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    admin_json_response(request, state, || {
        let Some(run_id) = run_id_from_path(request.path()) else {
            return Err(OpenAiRouteError::not_found_kind("run", request.path()));
        };
        run_get_expr(state, run_id)
    })
}

/// Handles the admin events listing, returning every recorded event as JSON.
pub fn handle_admin_events(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    admin_json_response(request, state, || events_expr(state))
}

/// Handles the admin storage-stats report (object counts and latencies).
pub fn handle_admin_storage_stats(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    admin_json_response(request, state, || storage_stats_expr(state))
}

/// Handles the admin model-health report over the registered runners.
pub fn handle_admin_model_health(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    admin_json_response(request, state, || model_health_expr(state))
}

/// Handles the admin plan-cache statistics report.
pub fn handle_admin_cache_stats(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    admin_json_response(request, state, || cache_stats_expr(state))
}

/// Handles the admin capability report (admin capability id, key count, and
/// anonymous capability grants).
pub fn handle_admin_capability_report(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    admin_json_response(request, state, || capability_report_expr(state))
}

pub(crate) fn runs_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let store = state.store().lock().map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
    })?;
    Ok(list_expr(
        "openai-gateway/runs",
        store
            .runs()
            .into_iter()
            .map(|(id, run)| run_expr(&id, &run))
            .collect(),
    ))
}

pub(crate) fn run_get_expr(state: &GatewayRouteState, run_id: &str) -> RouteResult<Expr> {
    let store = state.store().lock().map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
    })?;
    store
        .run_by_id(run_id)
        .map(|(id, run)| run_expr(&id, &run))
        .ok_or_else(|| OpenAiRouteError::not_found_kind("run", run_id))
}

pub(crate) fn events_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let store = state.store().lock().map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
    })?;
    Ok(list_expr(
        "openai-gateway/events",
        store
            .events()
            .into_iter()
            .map(|(id, event)| event_expr(&id, &event))
            .collect(),
    ))
}

pub(crate) fn storage_stats_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let store = state.store().lock().map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
    })?;
    let counts = store.counts();
    let requests = store.requests();
    let events = store.events();
    let counters = AdminCounters::from_ledger(counts, &requests, &events);
    Ok(Expr::Map(vec![
        field(
            "object",
            Expr::String("openai-gateway/storage-stats".to_owned()),
        ),
        field("counters", counters.to_expr()),
        field("storage-object-counts", storage_counts_expr(counts)),
        field(
            "average-latency-ms-by-model-id",
            average_latency_expr(&events),
        ),
        field(
            "rejected-request-counts-by-error-code",
            Expr::Map(Vec::new()),
        ),
        field("capability-denial-counts", Expr::Map(Vec::new())),
    ]))
}

pub(crate) fn model_health_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let cards = state.runners().cards();
    Ok(Expr::Map(vec![
        field(
            "object",
            Expr::String("openai-gateway/model-health".to_owned()),
        ),
        field("backend", Expr::Symbol(Symbol::new("sim"))),
        field("fixture-health", Expr::Symbol(Symbol::new("ok"))),
        field("registered-runner-count", usize_expr(cards.len())),
        field(
            "registered-models",
            Expr::List(
                cards
                    .into_iter()
                    .map(|card| {
                        Expr::Map(vec![
                            field("model", Expr::String(card.model)),
                            field("provider", Expr::Symbol(card.provider)),
                            field("health", Expr::Symbol(Symbol::new("unknown"))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

pub(crate) fn cache_stats_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let cache = state.cache().lock().map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway cache lock failed: {err}"))
    })?;
    Ok(Expr::Map(vec![
        field(
            "object",
            Expr::String("openai-gateway/cache-stats".to_owned()),
        ),
        field("entry-count", usize_expr(cache.len())),
        field("hit-count", usize_expr(0)),
        field("miss-count", usize_expr(0)),
        field("hit-rate", float_expr(0.0)),
    ]))
}

pub(crate) fn capability_report_expr(state: &GatewayRouteState) -> RouteResult<Expr> {
    let keys = state
        .keys()
        .list_keys()
        .map_err(OpenAiRouteError::internal)?;
    let anonymous = state
        .keys()
        .effective_capabilities(&GatewayRequest::get("/"))
        .map_err(OpenAiRouteError::internal)?;
    Ok(Expr::Map(vec![
        field(
            "object",
            Expr::String("openai-gateway/capability-report".to_owned()),
        ),
        field(
            "admin-capability",
            Expr::String(OPENAI_GATEWAY_ADMIN_CAPABILITY.to_owned()),
        ),
        field("key-count", usize_expr(keys.len())),
        field(
            "anonymous-capability-count",
            usize_expr(anonymous.iter().count()),
        ),
        field("capability-denial-count", usize_expr(0)),
        field(
            "keys",
            Expr::List(keys.into_iter().map(|key| key.to_expr()).collect()),
        ),
    ]))
}

pub(crate) fn admin_expr_json(expr: &Expr) -> Value {
    match expr {
        Expr::Nil => Value::Null,
        Expr::Bool(value) => Value::Bool(*value),
        Expr::Number(value) => number_json(value),
        Expr::String(value) => Value::String(value.clone()),
        Expr::Symbol(symbol) | Expr::Local(symbol) => Value::String(symbol.to_string()),
        Expr::Bytes(bytes) => Value::String(hex_encode(bytes)),
        Expr::List(values) | Expr::Vector(values) | Expr::Set(values) | Expr::Block(values) => {
            Value::Array(values.iter().map(admin_expr_json).collect())
        }
        Expr::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(expr_key(key), admin_expr_json(value));
            }
            Value::Object(object)
        }
        other => Value::String(format!("{other:?}")),
    }
}

fn admin_json_response(
    request: &GatewayRequest,
    state: &GatewayRouteState,
    expr: impl FnOnce() -> RouteResult<Expr>,
) -> GatewayResponse {
    match has_admin_access(request, state) {
        Ok(true) => {}
        Ok(false) => {
            return OpenAiRouteError::forbidden(
                "admin route requires openai-gateway.admin",
                "capability_denied",
            )
            .into_response();
        }
        Err(error) => return error.into_response(),
    }
    expr()
        .map(|expr| GatewayResponse::json(200, admin_expr_json(&expr).to_string().into_bytes()))
        .unwrap_or_else(OpenAiRouteError::into_response)
}

fn has_admin_access(request: &GatewayRequest, state: &GatewayRouteState) -> RouteResult<bool> {
    let capabilities = state
        .keys()
        .effective_capabilities(request)
        .map_err(OpenAiRouteError::internal)?;
    Ok(capabilities.contains(&CapabilityName::new(OPENAI_GATEWAY_ADMIN_CAPABILITY)))
}

fn list_expr(object: &str, data: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        field("object", Expr::String(object.to_owned())),
        field("data", Expr::List(data)),
    ])
}

fn run_expr(content_id: &ContentId, run: &GatewayRun) -> Expr {
    Expr::Map(vec![
        field("object", Expr::String("openai-gateway/run".to_owned())),
        field("content-id", content_id_expr(content_id)),
        field("id", Expr::String(run.id().to_owned())),
        field(
            "request-content-id",
            content_id_expr(run.request_content_id()),
        ),
        field("status", Expr::Symbol(run.status().clone())),
        field("created-at-ms", u64_expr(run.created_at_ms())),
    ])
}

fn event_expr(content_id: &ContentId, event: &GatewayEvent) -> Expr {
    Expr::Map(vec![
        field("object", Expr::String("openai-gateway/event".to_owned())),
        field("content-id", content_id_expr(content_id)),
        field("id", Expr::String(event.id().to_owned())),
        field("run-id", Expr::String(event.run_id().to_owned())),
        field("sequence", u64_expr(event.sequence())),
        field("event-kind", Expr::Symbol(event.kind().clone())),
        field("created-at-ms", u64_expr(event.created_at_ms())),
        field("payload", event.payload().clone()),
    ])
}

fn storage_counts_expr(counts: GatewayStoreCounts) -> Expr {
    Expr::Map(vec![
        field("requests", usize_expr(counts.request_count)),
        field("runs", usize_expr(counts.run_count)),
        field("events", usize_expr(counts.event_count)),
        field("responses", usize_expr(counts.response_count)),
        field("response-objects", usize_expr(counts.response_object_count)),
        field("files", usize_expr(counts.file_count)),
        field("file-bytes", usize_expr(counts.file_bytes_count)),
        field("batches", usize_expr(counts.batch_count)),
        field("threads", usize_expr(counts.thread_count)),
        field("thread-messages", usize_expr(counts.thread_message_count)),
        field("vector-stores", usize_expr(counts.vector_store_count)),
    ])
}

fn average_latency_expr(events: &[(ContentId, GatewayEvent)]) -> Expr {
    let mut totals = BTreeMap::<String, (u64, u64)>::new();
    for (_, event) in events {
        if event.kind().name.as_ref() != "final" {
            continue;
        }
        let Some(model) = string_field(event.payload(), "model") else {
            continue;
        };
        let Some(usage) = map_field(event.payload(), "usage") else {
            continue;
        };
        let Some(latency) = u64_field(usage, "latency-ms") else {
            continue;
        };
        let entry = totals.entry(model.to_owned()).or_default();
        entry.0 = entry.0.saturating_add(latency);
        entry.1 = entry.1.saturating_add(1);
    }
    Expr::Map(
        totals
            .into_iter()
            .filter_map(|(model, (total, count))| {
                (count != 0).then(|| (Expr::String(model), u64_expr(total / count)))
            })
            .collect(),
    )
}

fn request_streams(request: &GatewayRequest) -> bool {
    serde_json::from_slice::<Value>(request.body())
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn run_id_from_path(path: &str) -> Option<&str> {
    super::path::id_from_path(path, ADMIN_RUN_RETRIEVAL_PREFIX)
}

fn string_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a str> {
    field_any(expr, name).and_then(|value| match value {
        Expr::String(value) => Some(value.as_str()),
        _ => None,
    })
}

fn map_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    field_any(expr, name).filter(|value| matches!(value, Expr::Map(_)))
}

fn u64_field(expr: &Expr, name: &str) -> Option<u64> {
    match field_any(expr, name)? {
        Expr::Number(number) => number.canonical.parse().ok(),
        Expr::String(value) => value.parse().ok(),
        _ => None,
    }
}

use sim_value::build::entry as field;

fn usize_expr(value: usize) -> Expr {
    number_expr("usize", value.to_string())
}

fn u64_expr(value: u64) -> Expr {
    number_expr("u64", value.to_string())
}

fn float_expr(value: f64) -> Expr {
    number_expr("f64", value.to_string())
}

fn number_expr(domain: &str, canonical: String) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::new(domain),
        canonical,
    })
}

fn number_json(value: &NumberLiteral) -> Value {
    value
        .canonical
        .parse::<i64>()
        .ok()
        .map(Number::from)
        .map(Value::Number)
        .or_else(|| {
            value
                .canonical
                .parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map(Value::Number)
        })
        .unwrap_or_else(|| Value::String(value.canonical.clone()))
}

fn expr_key(expr: &Expr) -> String {
    match expr {
        Expr::String(value) => value.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        _ => format!("{expr:?}"),
    }
}
