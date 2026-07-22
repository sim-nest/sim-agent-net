use serde_json::{Map, Value};

use super::errors::OpenAiRouteError;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

pub(crate) fn request_object(body: &[u8]) -> RouteResult<Map<String, Value>> {
    let value = serde_json::from_slice::<Value>(body).map_err(|err| {
        OpenAiRouteError::invalid_json(format!("invalid JSON request body: {err}"))
    })?;
    let Value::Object(object) = value else {
        return Err(OpenAiRouteError::bad_request(
            "request body must be a JSON object",
            None,
            "invalid_request",
        ));
    };
    Ok(object)
}

/// Like [`request_object`], but tolerates an empty/null body by returning an
/// empty object. Used by routes (such as thread creation) that accept a request
/// with no JSON body.
pub(crate) fn request_object_or_empty(body: &[u8]) -> RouteResult<Map<String, Value>> {
    if body.is_empty() {
        return Ok(Map::new());
    }
    let value = serde_json::from_slice::<Value>(body).map_err(|err| {
        OpenAiRouteError::invalid_json(format!("invalid JSON request body: {err}"))
    })?;
    match value {
        Value::Object(object) => Ok(object),
        Value::Null => Ok(Map::new()),
        _ => Err(OpenAiRouteError::bad_request(
            "request body must be a JSON object",
            None,
            "invalid_request",
        )),
    }
}

pub(crate) fn required_string<'a>(
    object: &'a Map<String, Value>,
    name: &'static str,
) -> RouteResult<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| OpenAiRouteError::missing_required(name))
}

pub(crate) fn optional_string<'a>(
    object: &'a Map<String, Value>,
    name: &'static str,
    default: &'a str,
) -> &'a str {
    object.get(name).and_then(Value::as_str).unwrap_or(default)
}

pub(crate) fn optional_u64(
    object: &Map<String, Value>,
    name: &'static str,
    default: u64,
) -> RouteResult<u64> {
    match object.get(name) {
        Some(value) => value.as_u64().ok_or_else(|| {
            OpenAiRouteError::bad_request(
                format!("{name} must be an unsigned integer"),
                Some(name),
                "invalid_request",
            )
        }),
        None => Ok(default),
    }
}

pub(crate) fn record_execution(object: &Map<String, Value>) -> bool {
    object.get("store").and_then(Value::as_bool).unwrap_or(true)
}
