use std::sync::Arc;

use serde_json::{Map, Value};
use sim_codec::Input;
use sim_kernel::{Cx, DefaultFactory, EvalFabric, Expr, NoopEvalPolicy};

use crate::{
    clock::{GatewayClock, SystemGatewayClock},
    codec_openai::{
        OpenAiSseSurface, decode_openai_request, encode_gateway_events_sse,
        encode_openai_responses_response,
    },
    content_id::{content_id_for_expr, request_content_id},
    objects::{GatewayRequest, GatewayResponse, GatewayResponseValue, GatewayRun},
    plan::{
        check_plan, eval_plan_report_with_cache, eval_plan_report_with_cache_and_runners,
        eval_plan_report_with_cache_runners_and_federation, parse_plan,
    },
    runtime::{
        OpenAiGatewayFabric, OpenAiPlanCache, OpenAiRunnerRegistry, redacted_gateway_request,
        run_tool_loop_with_cache,
    },
    server::GatewayRouteState,
    storage::{GatewayResponseObjectStore, GatewayStateStore, GatewayStore, StoredGatewayResponse},
};

pub use super::response_runtime::{
    ResponseExecution, ResponseIdGenerators, ResponseRuntimeTargets,
};
use super::{
    errors::OpenAiRouteError,
    response_log::{EventInput, EventLog, append_event, response_usage_expr},
    response_text::response_delta_chunks,
    thread_context::normalize_response_request,
};

/// Route path for the OpenAI-compatible `POST /v1/responses` endpoint.
pub const RESPONSES_PATH: &str = "/v1/responses";
/// Path prefix stripped to extract a response id from a retrieval request.
pub const RESPONSE_RETRIEVAL_PREFIX: &str = "/v1/responses/";
/// Route template for retrieving a single stored response by id.
pub const RESPONSE_RETRIEVAL_ROUTE: &str = "/v1/responses/{id}";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Handles `POST /v1/responses`, realizing the request through the gateway
/// eval fabric under the caller's effective capabilities.
pub fn handle_responses(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    let fabric = OpenAiGatewayFabric::with_state_system(state.clone(), seed);
    match state
        .keys()
        .with_effective_capabilities(&mut cx, request, |cx| {
            fabric.realize(
                cx,
                OpenAiGatewayFabric::eval_request_for_gateway_request(request),
            )
        }) {
        Ok(reply) => {
            let Some(response) = reply.value.object().downcast_ref::<GatewayResponseValue>() else {
                return OpenAiRouteError::internal_message(
                    "openai gateway fabric returned a non-response value",
                )
                .into_response();
            };
            response.response().clone()
        }
        Err(err) => OpenAiRouteError::internal_message(format!("gateway realize failed: {err}"))
            .into_response(),
    }
}

/// Handles `GET /v1/responses/{id}`, returning the stored response for the id
/// in the path.
pub fn handle_response_retrieval(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let Some(response_id) = response_id_from_path(request.path()) else {
        return OpenAiRouteError::not_found("response").into_response();
    };
    match state.store().lock() {
        Ok(store) => retrieve_response(&*store, response_id),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Returns the stored response object for `response_id`, or a not-found error
/// response if no such response is stored.
pub fn retrieve_response<S>(store: &S, response_id: &str) -> GatewayResponse
where
    S: GatewayResponseObjectStore,
{
    store
        .response_object(response_id)
        .map(|record| record.response().clone())
        .unwrap_or_else(|| OpenAiRouteError::not_found(response_id).into_response())
}

/// Executes a `/v1/responses` request with a fresh plan cache and no runner or
/// federation targets, returning the [`ResponseExecution`] outcome.
pub fn execute_response_request<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> ResponseExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    let mut cache = OpenAiPlanCache::new();
    execute_response_request_with_cache(cx, store, &mut cache, ids, clock, request)
}

/// Executes a `/v1/responses` request reusing the given plan cache, with no
/// runner or federation targets.
pub fn execute_response_request_with_cache<S, C>(
    cx: &mut Cx,
    store: &mut S,
    cache: &mut OpenAiPlanCache,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> ResponseExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    match try_execute_response_request(cx, store, cache, ids, clock, request, None) {
        Ok(execution) => execution,
        Err(error) => ResponseExecution::error(error),
    }
}

/// Executes a `/v1/responses` request against the given runner registry,
/// using a fresh plan cache and no federation.
pub fn execute_response_request_with_runners<S, C>(
    cx: &mut Cx,
    store: &mut S,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    runners: &OpenAiRunnerRegistry,
) -> ResponseExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    let mut cache = OpenAiPlanCache::new();
    execute_response_request_with_cache_and_runners(
        cx, store, &mut cache, ids, clock, request, runners,
    )
}

/// Executes a `/v1/responses` request against the given runner registry,
/// reusing the supplied plan cache and using no federation.
pub fn execute_response_request_with_cache_and_runners<S, C>(
    cx: &mut Cx,
    store: &mut S,
    cache: &mut OpenAiPlanCache,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    runners: &OpenAiRunnerRegistry,
) -> ResponseExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    match try_execute_response_request(
        cx,
        store,
        cache,
        ids,
        clock,
        request,
        Some(ResponseRuntimeTargets::runners(runners)),
    ) {
        Ok(execution) => execution,
        Err(error) => ResponseExecution::error(error),
    }
}

/// Executes a `/v1/responses` request reusing the given plan cache and
/// dispatching to the supplied runner and federation [`ResponseRuntimeTargets`].
pub fn execute_response_request_with_cache_runners_and_federation<S, C>(
    cx: &mut Cx,
    store: &mut S,
    cache: &mut OpenAiPlanCache,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    targets: ResponseRuntimeTargets<'_>,
) -> ResponseExecution
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    match try_execute_response_request(cx, store, cache, ids, clock, request, Some(targets)) {
        Ok(execution) => execution,
        Err(error) => ResponseExecution::error(error),
    }
}

fn try_execute_response_request<S, C>(
    cx: &mut Cx,
    store: &mut S,
    cache: &mut OpenAiPlanCache,
    ids: &mut ResponseIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    targets: Option<ResponseRuntimeTargets<'_>>,
) -> RouteResult<ResponseExecution>
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: GatewayClock,
{
    let normalized = normalize_response_request(store, request)?;
    let object = normalized.object;
    let model = required_string(&object, "model")?.to_owned();
    require_input(&object)?;
    let store_response = object
        .get("store")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stream_response = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let request_expr = decode_openai_request(Input::Bytes(normalized.request.body().to_vec()))
        .map_err(OpenAiRouteError::bad_request_from_error)?;
    let plan = parse_plan(&model).map_err(OpenAiRouteError::bad_model_from_error)?;
    check_plan(&plan).map_err(OpenAiRouteError::bad_model_from_error)?;

    let recorded_request = redacted_gateway_request(&normalized.request).with_metadata(
        ids.request.next_id().map_err(OpenAiRouteError::internal)?,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    let request_content_id =
        request_content_id(&recorded_request).map_err(OpenAiRouteError::internal)?;
    if store_response {
        store
            .put_request(request_content_id.clone(), recorded_request.clone())
            .map_err(OpenAiRouteError::internal)?;
    }

    let run_id = ids.run.next_id().map_err(OpenAiRouteError::internal)?;
    let run = GatewayRun::new(
        run_id.clone(),
        request_content_id.clone(),
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    let run_content_id = content_id_for_expr(&run.to_expr()).map_err(OpenAiRouteError::internal)?;
    if store_response {
        store
            .put_run(run_content_id.clone(), run.clone())
            .map_err(OpenAiRouteError::internal)?;
    }

    let mut event_log = EventLog::default();
    let mut sequence = 0;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(sequence, "request-start", recorded_request.to_expr()),
        store_response,
        &mut event_log,
    )?;
    sequence += 1;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(sequence, "plan-start", plan.clone()),
        store_response,
        &mut event_log,
    )?;
    sequence += 1;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(sequence, "model-start", Expr::String(model.clone())),
        store_response,
        &mut event_log,
    )?;
    sequence += 1;

    let initial_plan_report = match targets {
        Some(targets) => {
            if let Some(federation) = targets.federation_ref() {
                eval_plan_report_with_cache_runners_and_federation(
                    cx,
                    &plan,
                    &request_expr,
                    cache,
                    targets.runners_ref(),
                    federation,
                )
            } else {
                eval_plan_report_with_cache_and_runners(
                    cx,
                    &plan,
                    &request_expr,
                    cache,
                    targets.runners_ref(),
                )
            }
        }
        None => eval_plan_report_with_cache(cx, &plan, &request_expr, cache),
    }
    .map_err(|err| OpenAiRouteError::model(err, &model))?;
    let plan_report = run_tool_loop_with_cache(
        cx,
        &plan,
        &request_expr,
        &object,
        cache,
        initial_plan_report,
    )
    .map_err(|err| OpenAiRouteError::model(err, &model))?;
    for event in &plan_report.events {
        append_event(
            store,
            ids,
            clock,
            &run_id,
            EventInput::from_symbol(sequence, event.kind.clone(), event.payload.clone()),
            store_response,
            &mut event_log,
        )?;
        sequence += 1;
    }
    let model_response = plan_report.response;
    for delta in response_delta_chunks(&model_response, stream_response)? {
        append_event(
            store,
            ids,
            clock,
            &run_id,
            EventInput::new(sequence, "delta", Expr::String(delta)),
            store_response,
            &mut event_log,
        )?;
        sequence += 1;
    }
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(sequence, "usage", response_usage_expr(&model_response)),
        store_response,
        &mut event_log,
    )?;
    sequence += 1;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(sequence, "final", model_response.clone()),
        store_response,
        &mut event_log,
    )?;

    let response_id = ids.response.next_id().map_err(OpenAiRouteError::internal)?;
    let response_created_at = clock.now_ms().map_err(OpenAiRouteError::internal)?;
    let response_body =
        encode_openai_responses_response(&model_response, &response_id, response_created_at)
            .map_err(OpenAiRouteError::internal)?;
    let final_response = GatewayResponse::json(200, response_body);
    let response_content_id = if store_response {
        let id =
            content_id_for_expr(&final_response.to_expr()).map_err(OpenAiRouteError::internal)?;
        let mut record =
            StoredGatewayResponse::new(response_id.clone(), id.clone(), final_response.clone());
        record.request_content_id = Some(request_content_id.clone());
        record.run_content_id = Some(run_content_id.clone());
        record.event_content_ids = event_log.content_ids.clone();
        store
            .put_response_object(record)
            .map_err(OpenAiRouteError::internal)?;
        Some(id)
    } else {
        None
    };
    let response = if stream_response {
        GatewayResponse::sse(
            200,
            encode_gateway_events_sse(
                &event_log.events,
                OpenAiSseSurface::Responses,
                &response_id,
                response_created_at,
            )
            .map_err(OpenAiRouteError::internal)?,
        )
    } else {
        final_response
    };

    Ok(ResponseExecution {
        response,
        request_content_id: Some(request_content_id),
        run_content_id: Some(run_content_id),
        event_content_ids: event_log.content_ids,
        events: event_log.events,
        response_id: Some(response_id),
        response_created_at_ms: Some(response_created_at),
        response_content_id,
    })
}

fn response_id_from_path(path: &str) -> Option<&str> {
    path.strip_prefix(RESPONSE_RETRIEVAL_PREFIX)
        .filter(|response_id| !response_id.is_empty() && !response_id.contains('/'))
}

fn required_string<'a>(object: &'a Map<String, Value>, name: &'static str) -> RouteResult<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| OpenAiRouteError::missing_required(name))
}

fn require_input(object: &Map<String, Value>) -> RouteResult<()> {
    if object.contains_key("input") || object.contains_key("messages") {
        Ok(())
    } else {
        Err(OpenAiRouteError::missing_required("input"))
    }
}
