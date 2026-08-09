use serde_json::{Map, Value, json};

use crate::{
    objects::{GatewayRequest, canonical_json_bytes},
    storage::{GatewayStateStore, GatewayThreadMessage},
};

use super::errors::OpenAiRouteError;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

pub(crate) struct NormalizedResponseRequest {
    pub(crate) object: Map<String, Value>,
    pub(crate) request: GatewayRequest,
}

pub(crate) fn normalize_response_request<S>(
    store: &S,
    request: &GatewayRequest,
) -> RouteResult<NormalizedResponseRequest>
where
    S: GatewayStateStore,
{
    let object = request_object(request.body())?;
    let object = with_thread_messages(store, object)?;
    let body = canonical_json_bytes(Value::Object(object.clone()));
    Ok(NormalizedResponseRequest {
        object,
        request: GatewayRequest::new(
            request.method().to_owned(),
            request.path().to_owned(),
            request.headers().to_vec(),
            body,
        ),
    })
}

use crate::routes::request_json::request_object;

fn with_thread_messages<S>(
    store: &S,
    mut object: Map<String, Value>,
) -> RouteResult<Map<String, Value>>
where
    S: GatewayStateStore,
{
    let Some(thread_id) = object.get("thread_id").and_then(Value::as_str) else {
        return Ok(object);
    };
    if store.thread(thread_id).is_none() {
        return Err(OpenAiRouteError::not_found_kind("thread", thread_id));
    }
    let mut messages = store
        .thread_messages(thread_id)
        .iter()
        .map(thread_message_json)
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Ok(object);
    }
    if let Some(Value::Array(existing)) = object.remove("messages") {
        messages.extend(existing);
        object.insert("messages".to_owned(), Value::Array(messages));
        return Ok(object);
    }
    let input = object
        .remove("input")
        .ok_or_else(|| OpenAiRouteError::missing_required("input"))?;
    let input = input.as_str().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "threaded response input must be a string",
            Some("input"),
            "invalid_input",
        )
    })?;
    messages.push(json!({ "role": "user", "content": input }));
    object.insert("messages".to_owned(), Value::Array(messages));
    Ok(object)
}

fn thread_message_json(message: &GatewayThreadMessage) -> Value {
    json!({
        "role": message.role(),
        "content": message.content(),
    })
}
