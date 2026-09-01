use std::sync::Arc;

use serde_json::{Map, Value, json};
use sim_kernel::{
    CapabilityName, CapabilitySet, ContentId, Cx, DefaultFactory, Expr, NoopEvalPolicy,
};
use sim_lib_net_core::hex_encode;

use crate::{
    capabilities::OPENAI_GATEWAY_ADMIN_CAPABILITY,
    clock::{SystemWallClock, WallClock},
    codec_openai::gateway_event_data_packets,
    objects::{GatewayEvent, GatewayRequest, GatewayResponse, content_id_hex},
    routes::responses::{RESPONSE_RETRIEVAL_PREFIX, ResponseIdGenerators, ResponseRuntimeTargets},
    runtime::OpenAiPlanCache,
    runtime::grant_capability_set,
    server::GatewayRouteState,
    storage::{GatewayResponseObjectStore, GatewayStateStore, GatewayStore, StoredGatewayResponse},
};

use super::{
    errors::OpenAiRouteError, responses::execute_response_request_with_cache_runners_and_federation,
};

/// Route template for retrieving a stored response's event history.
pub const RESPONSE_EVENTS_ROUTE: &str = "/v1/responses/{id}/events";
/// Path suffix marking a response event-history request.
pub const RESPONSE_EVENTS_SUFFIX: &str = "/events";
/// Route template for the SIM inspection view of a stored response.
pub const RESPONSE_SIM_ROUTE: &str = "/v1/responses/{id}/sim";
/// Path suffix marking a SIM inspection request.
pub const RESPONSE_SIM_SUFFIX: &str = "/sim";
/// Route path for replaying a stored response's recorded event stream.
pub const SIM_REPLAY_PATH: &str = "/v1/sim/replay";
/// Route path for forking a stored response with a request patch.
pub const SIM_FORK_PATH: &str = "/v1/sim/fork";
/// Capability id granting access to SIM extension routes.
pub const SIM_EXTENSION_CAPABILITY: &str = "sim.extension";
/// Capability id granting unredacted SIM response inspection.
pub const SIM_INSPECTION_CAPABILITY: &str = "sim.extension.inspect";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SimRouteAccess {
    caller_key_id: Option<String>,
    inspect: bool,
}

struct ForkExecutionContext<'a> {
    targets: ResponseRuntimeTargets<'a>,
    capabilities: &'a CapabilitySet,
    access: &'a SimRouteAccess,
}

/// Handles `GET /v1/responses/{id}/events`, returning the stored event
/// history for the response id in the path.
pub fn handle_response_events(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let Some(response_id) = suffixed_response_id(request.path(), RESPONSE_EVENTS_SUFFIX) else {
        return OpenAiRouteError::not_found_kind("response", request.path()).into_response();
    };
    let access = match sim_route_access(request, state) {
        Ok(access) => access,
        Err(error) => return error.into_response(),
    };
    match state.store().lock() {
        Ok(store) => response_events_with_access(&*store, response_id, &access),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `GET /v1/responses/{id}/sim`, returning the SIM inspection view of
/// a stored response; requires the admin or `sim.extension` capability.
pub fn handle_response_sim(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let Some(response_id) = suffixed_response_id(request.path(), RESPONSE_SIM_SUFFIX) else {
        return OpenAiRouteError::not_found_kind("response", request.path()).into_response();
    };
    let access = match sim_route_access(request, state) {
        Ok(access) => access,
        Err(error) => return error.into_response(),
    };
    match state.store().lock() {
        Ok(store) => response_sim_with_access(&*store, response_id, &access),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/sim/replay`, replaying the stored event stream for the
/// `response_id` carried in the request body.
pub fn handle_sim_replay(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let object = match request_object(request.body()) {
        Ok(object) => object,
        Err(error) => return error.into_response(),
    };
    let response_id = match required_string(&object, "response_id") {
        Ok(response_id) => response_id.to_owned(),
        Err(error) => return error.into_response(),
    };
    let access = match sim_route_access(request, state) {
        Ok(access) => access,
        Err(error) => return error.into_response(),
    };
    match state.store().lock() {
        Ok(store) => replay_response_with_access(&*store, &response_id, &access),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/sim/fork`, re-running a stored response's request with a
/// JSON `patch` applied and recording the result as a child response.
pub fn handle_sim_fork(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let object = match request_object(request.body()) {
        Ok(object) => object,
        Err(error) => return error.into_response(),
    };
    let response_id = match required_string(&object, "response_id") {
        Ok(response_id) => response_id.to_owned(),
        Err(error) => return error.into_response(),
    };
    let patch = match object.get("patch").and_then(Value::as_object) {
        Some(patch) => patch.clone(),
        None => return OpenAiRouteError::missing_required("patch").into_response(),
    };
    let access = match sim_route_access(request, state) {
        Ok(access) => access,
        Err(error) => return error.into_response(),
    };
    let mut clock = SystemWallClock;
    let seed = clock.now_ms().unwrap_or(1).saturating_add(1_000_000);
    let mut ids = ResponseIdGenerators::deterministic(seed);
    let targets = ResponseRuntimeTargets::with_federation(state.runners(), state.federation());
    let capabilities = match state.keys().effective_capabilities(request) {
        Ok(capabilities) => capabilities,
        Err(err) => {
            return OpenAiRouteError::internal_message(format!(
                "gateway key capability lookup failed: {err}"
            ))
            .into_response();
        }
    };
    match state.store().lock() {
        Ok(mut store) => fork_response(
            &mut *store,
            &mut ids,
            &mut clock,
            &response_id,
            patch,
            ForkExecutionContext {
                targets,
                capabilities: &capabilities,
                access: &access,
            },
        )
        .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Returns the stored event history for `response_id` as an OpenAI-style
/// `list` JSON response, or an error response if the response is unknown.
pub fn response_events<S>(store: &S, response_id: &str) -> GatewayResponse
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    stored_events(store, response_id)
        .map(|(record, events)| {
            GatewayResponse::json_value(200, event_history_json(&record, &events, true))
        })
        .unwrap_or_else(OpenAiRouteError::into_response)
}

fn response_events_with_access<S>(
    store: &S,
    response_id: &str,
    access: &SimRouteAccess,
) -> GatewayResponse
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    stored_events(store, response_id)
        .and_then(|(record, events)| {
            ensure_response_owner(&record, access)?;
            Ok(GatewayResponse::json_value(
                200,
                event_history_json(&record, &events, access.inspect),
            ))
        })
        .unwrap_or_else(OpenAiRouteError::into_response)
}

/// Returns the SIM inspection view of a stored response (request, run, events,
/// and content ids) as JSON, or an error response if the response is unknown.
pub fn response_sim<S>(store: &S, response_id: &str) -> GatewayResponse
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    stored_events(store, response_id)
        .and_then(|(record, events)| sim_json(store, &record, &events, true))
        .map(|value| GatewayResponse::json_value(200, value))
        .unwrap_or_else(OpenAiRouteError::into_response)
}

fn response_sim_with_access<S>(
    store: &S,
    response_id: &str,
    access: &SimRouteAccess,
) -> GatewayResponse
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    stored_events(store, response_id)
        .and_then(|(record, events)| {
            ensure_response_owner(&record, access)?;
            sim_json(store, &record, &events, access.inspect)
        })
        .map(|value| GatewayResponse::json_value(200, value))
        .unwrap_or_else(OpenAiRouteError::into_response)
}

fn replay_response_with_access<S>(
    store: &S,
    response_id: &str,
    access: &SimRouteAccess,
) -> GatewayResponse
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    stored_events(store, response_id)
        .and_then(|(record, events)| {
            ensure_response_owner(&record, access)?;
            Ok(GatewayResponse::json_value(
                200,
                json!({
                    "object": "sim.replay",
                    "response_id": record.response_id(),
                    "data": events_json(&record, &events, access.inspect),
                    "stream": data_stream_json(&events, access.inspect),
                }),
            ))
        })
        .unwrap_or_else(OpenAiRouteError::into_response)
}

fn fork_response<S, C>(
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    parent_response_id: &str,
    patch: Map<String, Value>,
    context: ForkExecutionContext<'_>,
) -> RouteResult<GatewayResponse>
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    let parent = store
        .response_object(parent_response_id)
        .ok_or_else(|| OpenAiRouteError::not_found(parent_response_id))?;
    ensure_response_owner(&parent, context.access)?;
    let source_request_id = parent.request_content_id.clone().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "stored response has no request ledger for fork",
            Some("response_id"),
            "missing_request_ledger",
        )
    })?;
    let source_request = store.request(&source_request_id).ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "stored response request is unavailable",
            Some("response_id"),
            "missing_request",
        )
    })?;
    let forked_request = forked_request(&source_request, patch)?;
    let (mut cx, seat) = Cx::new_seated(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x0A11_CE00),
    );
    grant_capability_set(&seat, &mut cx, context.capabilities)
        .map_err(OpenAiRouteError::internal)?;
    let mut cache = OpenAiPlanCache::new();
    let execution = execute_response_request_with_cache_runners_and_federation(
        &mut cx,
        store,
        &mut cache,
        ids,
        clock,
        &forked_request,
        context.targets,
    );
    if !(200..300).contains(&execution.response().status()) {
        return Ok(execution.response().clone());
    }
    let response_id = execution
        .response_id()
        .ok_or_else(|| OpenAiRouteError::internal_message("fork did not produce a response id"))?
        .to_owned();
    if let Some(mut record) = store.response_object(&response_id) {
        record.parent_response_id = Some(parent_response_id.to_owned());
        record.owner_key_id = context.access.caller_key_id.clone();
        store
            .put_response_object(record)
            .map_err(OpenAiRouteError::internal)?;
    }
    let request_content_id = execution.request_content_id().ok_or_else(|| {
        OpenAiRouteError::internal_message("fork did not produce a request content id")
    })?;
    let request_id = store
        .request(request_content_id)
        .and_then(|request| request.id().map(str::to_owned));
    Ok(GatewayResponse::json_value(
        200,
        json!({
            "object": "sim.fork",
            "parent_response_id": parent_response_id,
            "response_id": response_id,
            "request_id": request_id,
            "source_request_content_id": content_id_hex(&source_request_id),
            "request_content_id": content_id_hex(request_content_id),
            "response": response_body_json(execution.response())?,
        }),
    ))
}

fn stored_events<S>(
    store: &S,
    response_id: &str,
) -> RouteResult<(StoredGatewayResponse, Vec<GatewayEvent>)>
where
    S: GatewayResponseObjectStore + GatewayStore,
{
    let record = store
        .response_object(response_id)
        .ok_or_else(|| OpenAiRouteError::not_found(response_id))?;
    let events = record
        .event_content_ids
        .iter()
        .map(|id| {
            store.event(id).ok_or_else(|| {
                OpenAiRouteError::internal_message(format!(
                    "stored event is missing for response {response_id}: {}",
                    content_id_hex(id)
                ))
            })
        })
        .collect::<RouteResult<Vec<_>>>()?;
    Ok((record, events))
}

fn event_history_json(
    record: &StoredGatewayResponse,
    events: &[GatewayEvent],
    inspect: bool,
) -> Value {
    json!({
        "object": "list",
        "response_id": record.response_id(),
        "data": events_json(record, events, inspect),
    })
}

fn events_json(
    record: &StoredGatewayResponse,
    events: &[GatewayEvent],
    inspect: bool,
) -> Vec<Value> {
    record
        .event_content_ids
        .iter()
        .zip(events)
        .map(|(content_id, event)| event_json(content_id, event, inspect))
        .collect()
}

fn data_stream_json(events: &[GatewayEvent], inspect: bool) -> Vec<Value> {
    if !inspect {
        return Vec::new();
    }
    gateway_event_data_packets(events)
        .iter()
        .map(|packet| expr_json(&packet.to_expr()))
        .collect()
}

fn event_json(content_id: &ContentId, event: &GatewayEvent, inspect: bool) -> Value {
    json!({
        "id": event.id(),
        "object": "response.event",
        "type": format!("response.{}", event.kind().name.as_ref()),
        "event": event.kind().name.as_ref(),
        "run_id": event.run_id(),
        "sequence": event.sequence(),
        "created_at": event.created_at_ms(),
        "content_id": content_id_hex(content_id),
        "payload": if inspect { expr_json(event.payload()) } else { redacted_json() },
    })
}

fn sim_json<S>(
    store: &S,
    record: &StoredGatewayResponse,
    events: &[GatewayEvent],
    inspect: bool,
) -> RouteResult<Value>
where
    S: GatewayStore,
{
    let request = record
        .request_content_id
        .as_ref()
        .and_then(|id| store.request(id));
    let run = record.run_content_id.as_ref().and_then(|id| store.run(id));
    Ok(json!({
        "object": "sim.gateway.response",
        "response_id": record.response_id(),
        "parent_response_id": record.parent_response_id.as_deref(),
        "response_content_id": content_id_hex(record.content_id()),
        "request_content_id": record.request_content_id.as_ref().map(content_id_hex),
        "run_content_id": record.run_content_id.as_ref().map(content_id_hex),
        "event_content_ids": record.event_content_ids.iter().map(content_id_hex).collect::<Vec<_>>(),
        "request": request.as_ref().map(|request| request_json(request, inspect)),
        "run": run.as_ref().map(|run| expr_json(&run.to_expr())),
        "events": events.iter().map(|event| event_expr_json(event, inspect)).collect::<Vec<_>>(),
        "response": response_body_json(record.response())?,
    }))
}

fn request_json(request: &GatewayRequest, inspect: bool) -> Value {
    if inspect {
        return expr_json(&request.to_expr());
    }
    json!({
        "object": "openai-gateway/request",
        "id": request.id(),
        "timestamp-ms": request.timestamp_ms(),
        "method": request.method(),
        "path": request.path(),
        "headers": redacted_json(),
        "body": redacted_json(),
    })
}

fn event_expr_json(event: &GatewayEvent, inspect: bool) -> Value {
    if inspect {
        return expr_json(&event.to_expr());
    }
    json!({
        "object": "openai-gateway/event",
        "id": event.id(),
        "run-id": event.run_id(),
        "sequence": event.sequence(),
        "event-kind": event.kind().name.as_ref(),
        "created-at-ms": event.created_at_ms(),
        "payload": redacted_json(),
    })
}

fn redacted_json() -> Value {
    Value::String("[redacted]".to_owned())
}

fn forked_request(
    source: &GatewayRequest,
    patch: Map<String, Value>,
) -> RouteResult<GatewayRequest> {
    let mut object = request_object(source.body())?;
    for (key, value) in patch {
        object.insert(key, value);
    }
    object.insert("store".to_owned(), Value::Bool(true));
    object.insert("stream".to_owned(), Value::Bool(false));
    let body = crate::objects::canonical_json_bytes(Value::Object(object));
    Ok(GatewayRequest::new(
        source.method().to_owned(),
        source.path().to_owned(),
        source.headers().to_vec(),
        body,
    ))
}

use crate::routes::request_json::{request_object, required_string};

fn response_body_json(response: &GatewayResponse) -> RouteResult<Value> {
    serde_json::from_slice(response.body()).map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway response body is not JSON: {err}"))
    })
}

fn sim_route_access(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> RouteResult<SimRouteAccess> {
    let capabilities = state
        .keys()
        .effective_capabilities(request)
        .map_err(OpenAiRouteError::internal)?;
    let has_admin = has_capability(&capabilities, OPENAI_GATEWAY_ADMIN_CAPABILITY);
    let has_extension = capabilities.iter().any(is_sim_capability);
    if !has_admin && !has_extension {
        return Err(OpenAiRouteError::forbidden(
            "SIM response routes require openai-gateway.admin or sim.extension",
            "capability_denied",
        ));
    }
    let caller_key_id = state
        .keys()
        .key_for_request(request)
        .map_err(OpenAiRouteError::internal)?
        .map(|key| key.id().to_owned());
    Ok(SimRouteAccess {
        caller_key_id,
        inspect: has_admin || has_capability(&capabilities, SIM_INSPECTION_CAPABILITY),
    })
}

fn ensure_response_owner(
    record: &StoredGatewayResponse,
    access: &SimRouteAccess,
) -> RouteResult<()> {
    if access.inspect || record.owner_key_id() == access.caller_key_id.as_deref() {
        return Ok(());
    }
    Err(OpenAiRouteError::forbidden(
        "stored response belongs to a different gateway key",
        "capability_denied",
    ))
}

fn has_capability(capabilities: &CapabilitySet, capability: &str) -> bool {
    capabilities.contains(&CapabilityName::new(capability))
}

fn is_sim_capability(capability: &CapabilityName) -> bool {
    let capability = capability.as_str();
    capability == OPENAI_GATEWAY_ADMIN_CAPABILITY
        || capability == SIM_EXTENSION_CAPABILITY
        || capability.starts_with("sim.extension.")
}

fn suffixed_response_id<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    super::path::id_from_path_with_suffix(path, RESPONSE_RETRIEVAL_PREFIX, suffix)
}

fn expr_json(expr: &Expr) -> Value {
    match expr {
        Expr::Nil => Value::Null,
        Expr::Bool(value) => Value::Bool(*value),
        Expr::String(value) => Value::String(value.clone()),
        Expr::Number(value) => Value::String(format!("{value:?}")),
        Expr::Symbol(symbol) => Value::String(symbol.name.as_ref().to_owned()),
        Expr::Bytes(bytes) => Value::String(hex_encode(bytes)),
        Expr::List(values) | Expr::Vector(values) | Expr::Set(values) | Expr::Block(values) => {
            Value::Array(values.iter().map(expr_json).collect())
        }
        Expr::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(expr_key(key), expr_json(value));
            }
            Value::Object(object)
        }
        other => Value::String(format!("{other:?}")),
    }
}

fn expr_key(expr: &Expr) -> String {
    match expr {
        Expr::String(value) => value.clone(),
        Expr::Symbol(symbol) => symbol.name.as_ref().to_owned(),
        _ => format!("{expr:?}"),
    }
}
