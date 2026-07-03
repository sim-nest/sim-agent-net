use serde_json::{Map, Value, json};

use crate::{
    clock::{GatewayClock, SystemGatewayClock},
    ids::GatewayIdGenerator,
    objects::{GatewayRequest, GatewayResponse},
    server::GatewayRouteState,
    storage::{GatewayStateStore, GatewayThread, GatewayThreadMessage},
};

use super::errors::OpenAiRouteError;

/// Route path for thread creation (`POST /v1/threads`).
pub const THREADS_PATH: &str = "/v1/threads";
/// Path prefix shared by thread retrieval and message routes (`/v1/threads/`).
pub const THREAD_RETRIEVAL_PREFIX: &str = "/v1/threads/";
/// Templated route for retrieving a single thread by id (`/v1/threads/{id}`).
pub const THREAD_RETRIEVAL_ROUTE: &str = "/v1/threads/{id}";
/// Templated route for a thread's message collection (`/v1/threads/{id}/messages`).
pub const THREAD_MESSAGES_ROUTE: &str = "/v1/threads/{id}/messages";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Handles `POST /v1/threads`, creating a new thread and returning its JSON object.
pub fn handle_threads(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = GatewayIdGenerator::deterministic("thread", seed);
    match state.store().lock() {
        Ok(mut store) => create_thread(&mut *store, &mut ids, &mut clock, request)
            .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/threads/{id}/messages`, appending a message to the thread.
pub fn handle_thread_post(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let Some(thread_id) = message_thread_id_from_path(request.path()) else {
        return OpenAiRouteError::not_found_kind("thread", request.path()).into_response();
    };
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = GatewayIdGenerator::deterministic("msg", seed);
    match state.store().lock() {
        Ok(mut store) => append_message(&mut *store, &mut ids, &mut clock, thread_id, request)
            .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `GET /v1/threads/{id}` and `GET /v1/threads/{id}/messages`, dispatching
/// to thread retrieval or message listing based on the request path.
pub fn handle_thread_get(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    match thread_path(request.path()) {
        Some(ThreadPath::Thread(thread_id)) => match state.store().lock() {
            Ok(store) => retrieve_thread(&*store, thread_id),
            Err(err) => {
                OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
                    .into_response()
            }
        },
        Some(ThreadPath::Messages(thread_id)) => match state.store().lock() {
            Ok(store) => list_messages(&*store, thread_id),
            Err(err) => {
                OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
                    .into_response()
            }
        },
        None => OpenAiRouteError::not_found_kind("thread", request.path()).into_response(),
    }
}

/// Returns the JSON object for a stored thread, or a not-found error response.
pub fn retrieve_thread<S>(store: &S, thread_id: &str) -> GatewayResponse
where
    S: GatewayStateStore,
{
    store
        .thread(thread_id)
        .map(|thread| GatewayResponse::json(200, thread_json(&thread).to_string().into_bytes()))
        .unwrap_or_else(|| OpenAiRouteError::not_found_kind("thread", thread_id).into_response())
}

/// Returns the thread's messages as an OpenAI `list` object, or a not-found error
/// response when the thread does not exist.
pub fn list_messages<S>(store: &S, thread_id: &str) -> GatewayResponse
where
    S: GatewayStateStore,
{
    if store.thread(thread_id).is_none() {
        return OpenAiRouteError::not_found_kind("thread", thread_id).into_response();
    }
    let data = store
        .thread_messages(thread_id)
        .iter()
        .map(message_json)
        .collect::<Vec<_>>();
    GatewayResponse::json(
        200,
        json!({ "object": "list", "data": data })
            .to_string()
            .into_bytes(),
    )
}

fn create_thread<S, C>(
    store: &mut S,
    ids: &mut GatewayIdGenerator,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<GatewayResponse>
where
    S: GatewayStateStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let thread = GatewayThread::new(
        ids.next_id().map_err(OpenAiRouteError::internal)?,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
        metadata(object.get("metadata"))?,
    );
    store
        .put_thread(thread.clone())
        .map_err(OpenAiRouteError::internal)?;
    Ok(GatewayResponse::json(
        200,
        thread_json(&thread).to_string().into_bytes(),
    ))
}

fn append_message<S, C>(
    store: &mut S,
    ids: &mut GatewayIdGenerator,
    clock: &mut C,
    thread_id: &str,
    request: &GatewayRequest,
) -> RouteResult<GatewayResponse>
where
    S: GatewayStateStore,
    C: GatewayClock,
{
    if store.thread(thread_id).is_none() {
        return Err(OpenAiRouteError::not_found_kind("thread", thread_id));
    }
    let object = request_object(request.body())?;
    let role = required_string(&object, "role")?.to_owned();
    let content = required_string(&object, "content")?.to_owned();
    let message = GatewayThreadMessage::new(
        ids.next_id().map_err(OpenAiRouteError::internal)?,
        thread_id,
        role,
        content,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    store
        .put_thread_message(message.clone())
        .map_err(OpenAiRouteError::internal)?;
    Ok(GatewayResponse::json(
        200,
        message_json(&message).to_string().into_bytes(),
    ))
}

use crate::routes::request_json::request_object_or_empty as request_object;

fn required_string<'a>(object: &'a Map<String, Value>, name: &'static str) -> RouteResult<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| OpenAiRouteError::missing_required(name))
}

fn metadata(value: Option<&Value>) -> RouteResult<Vec<(String, String)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let object = value.as_object().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "metadata must be an object",
            Some("metadata"),
            "invalid_metadata",
        )
    })?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or_else(|| {
                    OpenAiRouteError::bad_request(
                        "metadata values must be strings",
                        Some("metadata"),
                        "invalid_metadata",
                    )
                })
        })
        .collect()
}

enum ThreadPath<'a> {
    Thread(&'a str),
    Messages(&'a str),
}

fn thread_path(path: &str) -> Option<ThreadPath<'_>> {
    let rest = path.strip_prefix(THREAD_RETRIEVAL_PREFIX)?;
    if let Some(thread_id) = rest.strip_suffix("/messages") {
        return valid_thread_id(thread_id).map(ThreadPath::Messages);
    }
    valid_thread_id(rest).map(ThreadPath::Thread)
}

fn message_thread_id_from_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(THREAD_RETRIEVAL_PREFIX)?;
    rest.strip_suffix("/messages").and_then(valid_thread_id)
}

fn valid_thread_id(thread_id: &str) -> Option<&str> {
    (!thread_id.is_empty() && !thread_id.contains('/')).then_some(thread_id)
}

fn thread_json(thread: &GatewayThread) -> Value {
    json!({
        "id": thread.id(),
        "object": "thread",
        "created_at": thread.created_at_ms(),
        "metadata": metadata_json(thread.metadata()),
    })
}

fn message_json(message: &GatewayThreadMessage) -> Value {
    json!({
        "id": message.id(),
        "object": "thread.message",
        "thread_id": message.thread_id(),
        "role": message.role(),
        "content": message.content(),
        "created_at": message.created_at_ms(),
    })
}

fn metadata_json(metadata: &[(String, String)]) -> Value {
    Value::Object(
        metadata
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect(),
    )
}
