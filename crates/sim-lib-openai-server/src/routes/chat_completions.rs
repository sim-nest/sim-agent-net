use std::sync::Arc;

use serde_json::{Map, Value, json};
use sim_kernel::{Cx, DefaultFactory, NoopEvalPolicy};

use crate::{
    clock::{SystemWallClock, WallClock},
    codec_openai::{OpenAiSseSurface, encode_gateway_events_sse},
    objects::{GatewayRequest, GatewayResponse},
    routes::responses::{
        RESPONSES_PATH, ResponseExecution, ResponseIdGenerators, ResponseRuntimeTargets,
        execute_response_request, execute_response_request_with_cache_runners_and_federation,
        execute_response_request_with_runners,
    },
    runtime::{OpenAiPlanCache, OpenAiRunnerRegistry},
    server::GatewayRouteState,
    storage::{GatewayResponseObjectStore, GatewayStateStore, GatewayStore},
};

use super::errors::OpenAiRouteError;

/// Route path for the chat completions endpoint (`POST /v1/chat/completions`).
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Outcome of a chat completion request: the gateway response plus the optional
/// underlying responses-runtime execution it was derived from.
#[derive(Clone, Debug)]
pub struct ChatCompletionExecution {
    response: GatewayResponse,
    runtime: Option<ResponseExecution>,
}

impl ChatCompletionExecution {
    /// Returns the gateway response produced for the chat completion request.
    pub fn response(&self) -> &GatewayResponse {
        &self.response
    }

    /// Returns the underlying responses-runtime execution, if one ran.
    pub fn runtime(&self) -> Option<&ResponseExecution> {
        self.runtime.as_ref()
    }

    fn error(error: OpenAiRouteError) -> Self {
        Self {
            response: error.into_response(),
            runtime: None,
        }
    }
}

/// Handles `POST /v1/chat/completions`, normalizing the request onto the responses
/// runtime under the caller's effective capabilities and returning the response.
pub fn handle_chat_completions(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let mut clock = SystemWallClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = ResponseIdGenerators::deterministic(seed);
    let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    match state.store().lock() {
        Ok(mut store) => match state
            .keys()
            .with_effective_capabilities(&mut cx, request, |cx| {
                Ok(execute_chat_completion_request_with_runners_and_federation(
                    cx,
                    &mut *store,
                    &mut ids,
                    &mut clock,
                    request,
                    ResponseRuntimeTargets::with_federation(state.runners(), state.federation()),
                ))
            }) {
            Ok(execution) => execution.response().clone(),
            Err(err) => OpenAiRouteError::internal_message(format!(
                "gateway key capability lookup failed: {err}"
            ))
            .into_response(),
        },
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Executes a chat completion request against the responses runtime without
/// runner or federation targets, capturing any error as a response.
pub fn execute_chat_completion_request<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> ChatCompletionExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    match try_execute_chat_completion_request(cx, store, ids, clock, request, None, None) {
        Ok(execution) => execution,
        Err(error) => ChatCompletionExecution::error(error),
    }
}

/// Executes a chat completion request with a runner registry available for
/// local model inference.
pub fn execute_chat_completion_request_with_runners<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    runners: &OpenAiRunnerRegistry,
) -> ChatCompletionExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    match try_execute_chat_completion_request(cx, store, ids, clock, request, Some(runners), None) {
        Ok(execution) => execution,
        Err(error) => ChatCompletionExecution::error(error),
    }
}

/// Executes a chat completion request with combined runner and federation
/// runtime targets.
pub fn execute_chat_completion_request_with_runners_and_federation<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    targets: ResponseRuntimeTargets<'_>,
) -> ChatCompletionExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    match try_execute_chat_completion_request(cx, store, ids, clock, request, None, Some(targets)) {
        Ok(execution) => execution,
        Err(error) => ChatCompletionExecution::error(error),
    }
}

fn try_execute_chat_completion_request<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    runners: Option<&OpenAiRunnerRegistry>,
    targets: Option<ResponseRuntimeTargets<'_>>,
) -> RouteResult<ChatCompletionExecution>
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    let stream_response = request_stream_flag(request.body())?;
    let runtime_request = chat_completion_runtime_request(request)?;
    let runtime = match (runners, targets) {
        (_, Some(targets)) => {
            let mut cache = OpenAiPlanCache::new();
            execute_response_request_with_cache_runners_and_federation(
                cx,
                store,
                &mut cache,
                ids,
                clock,
                &runtime_request,
                targets,
            )
        }
        (Some(runners), None) => {
            execute_response_request_with_runners(cx, store, ids, clock, &runtime_request, runners)
        }
        (None, _) => execute_response_request(cx, store, ids, clock, &runtime_request),
    };
    if runtime.response().status() != 200 {
        return Ok(ChatCompletionExecution {
            response: runtime.response().clone(),
            runtime: Some(runtime),
        });
    }
    let response = if stream_response {
        let response_id = runtime.response_id().ok_or_else(|| {
            OpenAiRouteError::internal_message("streaming runtime missing response id")
        })?;
        let created_at_ms = runtime.response_created_at_ms().ok_or_else(|| {
            OpenAiRouteError::internal_message("streaming runtime missing response timestamp")
        })?;
        GatewayResponse::sse(
            200,
            encode_gateway_events_sse(
                runtime.events(),
                OpenAiSseSurface::Chat,
                &chat_completion_id(response_id),
                created_at_ms,
            )
            .map_err(OpenAiRouteError::internal)?,
        )
    } else {
        chat_completion_response(runtime.response())?
    };
    Ok(ChatCompletionExecution {
        response,
        runtime: Some(runtime),
    })
}

pub(crate) fn chat_completion_runtime_request(
    request: &GatewayRequest,
) -> RouteResult<GatewayRequest> {
    let mut object = request_object(request.body())?;
    let input = {
        let messages = messages(&object)?;
        validate_supported_roles(messages)?;
        final_message_text(messages)?
    };
    object.insert("input".to_owned(), Value::String(input));
    let body = serde_json::to_vec(&Value::Object(object)).map_err(|err| {
        OpenAiRouteError::internal_message(format!(
            "failed to encode normalized chat completion request: {err}"
        ))
    })?;
    Ok(GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        request.headers().to_vec(),
        body,
    ))
}

fn request_stream_flag(body: &[u8]) -> RouteResult<bool> {
    Ok(request_object(body)?
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn chat_completion_response(response: &GatewayResponse) -> RouteResult<GatewayResponse> {
    let value = serde_json::from_slice::<Value>(response.body()).map_err(|err| {
        OpenAiRouteError::internal_message(format!(
            "responses runtime returned invalid json: {err}"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        OpenAiRouteError::internal_message("responses runtime returned a non-object response")
    })?;
    let response_id = string_member(object, "id")?;
    let created = object
        .get("created_at")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let model = string_member(object, "model")?;
    let output_text = string_member(object, "output_text")?;
    let usage = object.get("usage").cloned().unwrap_or(Value::Null);
    let body = serde_json::to_vec(&json!({
        "id": chat_completion_id(response_id),
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": output_text,
            },
            "finish_reason": "stop",
        }],
        "usage": usage,
    }))
    .map_err(|err| {
        OpenAiRouteError::internal_message(format!(
            "failed to encode chat completion response: {err}"
        ))
    })?;
    Ok(GatewayResponse::json(200, body))
}

use crate::routes::request_json::request_object;

fn messages(object: &Map<String, Value>) -> RouteResult<&[Value]> {
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| OpenAiRouteError::missing_required("messages"))?;
    if messages.is_empty() {
        return Err(OpenAiRouteError::bad_request(
            "chat completions messages must not be empty",
            Some("messages"),
            "invalid_messages",
        ));
    }
    Ok(messages)
}

fn validate_supported_roles(messages: &[Value]) -> RouteResult<()> {
    for message in messages {
        let role = message_role(message)?;
        if !matches!(role, "system" | "user" | "assistant") {
            return Err(OpenAiRouteError::bad_request(
                format!("unsupported chat message role: {role}"),
                Some("messages"),
                "unsupported_role",
            ));
        }
    }
    let final_role = message_role(messages.last().expect("messages is non-empty"))?;
    if final_role != "user" {
        return Err(OpenAiRouteError::bad_request(
            "final chat completion message must have role user",
            Some("messages"),
            "unsupported_role",
        ));
    }
    Ok(())
}

fn final_message_text(messages: &[Value]) -> RouteResult<String> {
    message_text(messages.last().expect("messages is non-empty"))
}

fn message_role(message: &Value) -> RouteResult<&str> {
    let object = message_object(message)?;
    object.get("role").and_then(Value::as_str).ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "chat completion message missing role",
            Some("messages"),
            "invalid_message",
        )
    })
}

fn message_text(message: &Value) -> RouteResult<String> {
    let object = message_object(message)?;
    content_text(object.get("content"))
}

fn message_object(message: &Value) -> RouteResult<&Map<String, Value>> {
    message.as_object().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "chat completion message must be an object",
            Some("messages"),
            "invalid_message",
        )
    })
}

fn content_text(content: Option<&Value>) -> RouteResult<String> {
    match content {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(text_part)
            .collect::<RouteResult<Vec<_>>>()
            .map(|items| items.join("\n")),
        Some(Value::Null) | None => Ok(String::new()),
        _ => Err(OpenAiRouteError::bad_request(
            "chat completion message content must be string, array, or null",
            Some("messages"),
            "invalid_message_content",
        )),
    }
}

fn text_part(part: &Value) -> RouteResult<String> {
    let object = part.as_object().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "chat completion content part must be an object",
            Some("messages"),
            "invalid_message_content",
        )
    })?;
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("text");
    match kind {
        "text" => object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                OpenAiRouteError::bad_request(
                    "chat completion text content part missing text",
                    Some("messages"),
                    "invalid_message_content",
                )
            }),
        other => Err(OpenAiRouteError::bad_request(
            format!("chat completion content part type {other} is not supported"),
            Some("messages"),
            "unsupported_content_part",
        )),
    }
}

fn string_member<'a>(object: &'a Map<String, Value>, name: &str) -> RouteResult<&'a str> {
    object.get(name).and_then(Value::as_str).ok_or_else(|| {
        OpenAiRouteError::internal_message(format!("responses runtime response missing {name}"))
    })
}

fn chat_completion_id(response_id: &str) -> String {
    match response_id.strip_prefix("resp_") {
        Some(suffix) => format!("chatcmpl_{suffix}"),
        None => format!("chatcmpl_{response_id}"),
    }
}
