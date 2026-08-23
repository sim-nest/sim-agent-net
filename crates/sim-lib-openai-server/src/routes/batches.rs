use std::sync::Arc;

use serde_json::{Map, Value, json};
use sim_kernel::{CapabilitySet, Cx, DefaultFactory, Expr, NoopEvalPolicy};

use crate::{
    clock::{SystemWallClock, WallClock},
    content_id::content_id_for_expr,
    ids::GatewayIdGenerator,
    objects::{GatewayRequest, GatewayResponse},
    routes::responses::{
        RESPONSES_PATH, ResponseIdGenerators, ResponseRuntimeTargets,
        execute_response_request_with_cache_runners_and_federation,
    },
    runtime::{OpenAiPlanCache, grant_capability_set},
    server::GatewayRouteState,
    storage::{
        GatewayBatch, GatewayBatchCounts, GatewayBatchStatus, GatewayFile, GatewayFileStorageRef,
        GatewayResponseObjectStore, GatewayStateStore, GatewayStore,
    },
};

use super::errors::OpenAiRouteError;

/// Route path for batch creation (`POST /v1/batches`).
pub const BATCHES_PATH: &str = "/v1/batches";
/// Path prefix shared by batch retrieval and cancel routes (`/v1/batches/`).
pub const BATCH_RETRIEVAL_PREFIX: &str = "/v1/batches/";
/// Templated route for retrieving a single batch by id (`/v1/batches/{id}`).
pub const BATCH_RETRIEVAL_ROUTE: &str = "/v1/batches/{id}";
/// Templated route for cancelling a batch (`/v1/batches/{id}/cancel`).
pub const BATCH_CANCEL_ROUTE: &str = "/v1/batches/{id}/cancel";
type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;
struct BatchIdGenerators {
    batch: GatewayIdGenerator,
    file: GatewayIdGenerator,
    response: ResponseIdGenerators,
}
impl BatchIdGenerators {
    fn deterministic(start: u64) -> Self {
        Self {
            batch: GatewayIdGenerator::deterministic("batch", start),
            file: GatewayIdGenerator::deterministic("file", start),
            response: ResponseIdGenerators::deterministic(start),
        }
    }
}
struct BatchItem {
    custom_id: String,
    method: String,
    path: String,
    body: Map<String, Value>,
}

struct BatchRunResult {
    output_records: Vec<Value>,
    error_records: Vec<Value>,
    completed: u64,
    failed: u64,
}

/// Handles `POST /v1/batches`, creating a batch and (unless deferred) running
/// its JSONL items through the responses runtime.
pub fn handle_batches(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemWallClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = BatchIdGenerators::deterministic(seed);
    let capabilities = match state.keys().effective_capabilities(request) {
        Ok(capabilities) => capabilities,
        Err(err) => return OpenAiRouteError::internal(err).into_response(),
    };
    match state.store().lock() {
        Ok(mut store) => create_batch(
            &mut *store,
            &mut ids,
            &mut clock,
            request,
            ResponseRuntimeTargets::with_federation(state.runners(), state.federation()),
            &capabilities,
        )
        .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `GET /v1/batches/{id}`, returning the stored batch object.
pub fn handle_batch_retrieval(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let Some(batch_id) = batch_id_from_path(request.path()) else {
        return OpenAiRouteError::not_found_kind("batch", request.path()).into_response();
    };
    match state.store().lock() {
        Ok(store) => retrieve_batch(&*store, batch_id),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/batches/{id}/cancel`, cancelling a queued or in-progress batch.
pub fn handle_batch_cancel(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let Some(batch_id) = cancel_batch_id_from_path(request.path()) else {
        return OpenAiRouteError::not_found_kind("batch", request.path()).into_response();
    };
    let mut clock = SystemWallClock;
    match state.store().lock() {
        Ok(mut store) => cancel_batch(&mut *store, &mut clock, batch_id)
            .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Returns the JSON object for a stored batch, or a not-found error response.
pub fn retrieve_batch<S>(store: &S, batch_id: &str) -> GatewayResponse
where
    S: GatewayStateStore,
{
    store
        .batch(batch_id)
        .map(|batch| GatewayResponse::json_value(200, batch_json(&batch)))
        .unwrap_or_else(|| OpenAiRouteError::not_found_kind("batch", batch_id).into_response())
}

fn create_batch<S, C>(
    store: &mut S,
    ids: &mut BatchIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    targets: ResponseRuntimeTargets<'_>,
    capabilities: &CapabilitySet,
) -> RouteResult<GatewayResponse>
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    let object = request_object(request.body())?;
    let input_file_id = required_string(&object, "input_file_id")?.to_owned();
    let endpoint = required_string(&object, "endpoint")?.to_owned();
    if endpoint != RESPONSES_PATH {
        return Err(OpenAiRouteError::bad_request(
            format!("unsupported batch endpoint: {endpoint}"),
            Some("endpoint"),
            "unsupported_endpoint",
        ));
    }
    let input_bytes = store
        .file_bytes(&input_file_id)
        .ok_or_else(|| OpenAiRouteError::not_found_kind("file", &input_file_id))?;
    let lines = jsonl_lines(&input_bytes)?;
    let created_at_ms = clock.now_ms().map_err(OpenAiRouteError::internal)?;
    let batch = GatewayBatch::new(
        ids.batch.next_id().map_err(OpenAiRouteError::internal)?,
        input_file_id,
        endpoint,
        created_at_ms,
        GatewayBatchCounts::new(lines.len() as u64, 0, 0, 0),
    );
    store
        .put_batch(batch.clone())
        .map_err(OpenAiRouteError::internal)?;
    if object
        .get("defer")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(GatewayResponse::json_value(200, batch_json(&batch)));
    }

    let result = run_batch_items(store, ids, clock, &batch, &lines, targets, capabilities)?;
    let output_file_id = store_jsonl_file(
        store,
        ids,
        clock,
        batch.id(),
        "output",
        "batch_output",
        &result.output_records,
    )?;
    let error_file_id = store_jsonl_file(
        store,
        ids,
        clock,
        batch.id(),
        "errors",
        "batch_error",
        &result.error_records,
    )?;
    let completed = batch.complete(
        output_file_id,
        error_file_id,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
        GatewayBatchCounts::new(
            result.completed + result.failed,
            result.completed,
            result.failed,
            0,
        ),
    );
    store
        .put_batch(completed.clone())
        .map_err(OpenAiRouteError::internal)?;
    Ok(GatewayResponse::json_value(200, batch_json(&completed)))
}

fn run_batch_items<S, C>(
    store: &mut S,
    ids: &mut BatchIdGenerators,
    clock: &mut C,
    batch: &GatewayBatch,
    lines: &[(usize, String)],
    targets: ResponseRuntimeTargets<'_>,
    capabilities: &CapabilitySet,
) -> RouteResult<BatchRunResult>
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
    C: WallClock,
{
    let (mut cx, seat) = Cx::new_seated(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0xBA7C_0001),
    );
    grant_capability_set(&seat, &mut cx, capabilities).map_err(OpenAiRouteError::internal)?;
    let mut cache = OpenAiPlanCache::new();
    let mut result = BatchRunResult {
        output_records: Vec::new(),
        error_records: Vec::new(),
        completed: 0,
        failed: 0,
    };
    for (line_number, line) in lines {
        match batch_item(line, batch.endpoint()) {
            Ok(item) => {
                let request = item_request(&item)?;
                let execution = execute_response_request_with_cache_runners_and_federation(
                    &mut cx,
                    store,
                    &mut cache,
                    &mut ids.response,
                    clock,
                    &request,
                    targets,
                );
                let response = execution.response();
                if (200..300).contains(&response.status()) {
                    result.completed += 1;
                    result.output_records.push(output_record(&item, response)?);
                } else {
                    result.failed += 1;
                    result.error_records.push(response_error_record(
                        &item.custom_id,
                        Some(response.status()),
                        response,
                    )?);
                }
            }
            Err(error) => {
                result.failed += 1;
                result
                    .error_records
                    .push(line_error_record(*line_number, error)?);
            }
        }
    }
    Ok(result)
}

fn cancel_batch<S, C>(store: &mut S, clock: &mut C, batch_id: &str) -> RouteResult<GatewayResponse>
where
    S: GatewayStateStore,
    C: WallClock,
{
    let batch = store
        .batch(batch_id)
        .ok_or_else(|| OpenAiRouteError::not_found_kind("batch", batch_id))?;
    let batch = if matches!(
        batch.status(),
        GatewayBatchStatus::Queued | GatewayBatchStatus::InProgress
    ) {
        batch.cancel(clock.now_ms().map_err(OpenAiRouteError::internal)?)
    } else {
        batch
    };
    store
        .put_batch(batch.clone())
        .map_err(OpenAiRouteError::internal)?;
    Ok(GatewayResponse::json_value(200, batch_json(&batch)))
}

use crate::routes::request_json::{request_object, required_string};

fn jsonl_lines(bytes: &[u8]) -> RouteResult<Vec<(usize, String)>> {
    let text = std::str::from_utf8(bytes).map_err(|err| {
        OpenAiRouteError::bad_request(
            format!("batch input file must be UTF-8 JSONL: {err}"),
            Some("input_file_id"),
            "invalid_batch_file",
        )
    })?;
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty()).then(|| (index + 1, line.to_owned()))
        })
        .collect())
}

fn batch_item(line: &str, endpoint: &str) -> RouteResult<BatchItem> {
    let value = serde_json::from_str::<Value>(line).map_err(|err| {
        OpenAiRouteError::invalid_json(format!("invalid JSONL batch line: {err}"))
    })?;
    let object = value.as_object().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "batch line must be a JSON object",
            None,
            "invalid_batch_line",
        )
    })?;
    let custom_id = required_string(object, "custom_id")?.to_owned();
    let method = required_string(object, "method")?.to_owned();
    if method != "POST" {
        return Err(OpenAiRouteError::bad_request(
            "batch item method must be POST",
            Some("method"),
            "unsupported_method",
        ));
    }
    let path = object
        .get("url")
        .or_else(|| object.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| OpenAiRouteError::missing_required("url"))?
        .to_owned();
    if path != endpoint {
        return Err(OpenAiRouteError::bad_request(
            format!("batch item endpoint mismatch: {path}"),
            Some("url"),
            "endpoint_mismatch",
        ));
    }
    let body = object
        .get("body")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| OpenAiRouteError::missing_required("body"))?;
    Ok(BatchItem {
        custom_id,
        method,
        path,
        body,
    })
}

fn item_request(item: &BatchItem) -> RouteResult<GatewayRequest> {
    let mut body = item.body.clone();
    body.insert("store".to_owned(), Value::Bool(true));
    body.insert("stream".to_owned(), Value::Bool(false));
    let body = crate::objects::canonical_json_bytes(Value::Object(body));
    Ok(GatewayRequest::new(
        item.method.clone(),
        item.path.clone(),
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body,
    ))
}

fn output_record(item: &BatchItem, response: &GatewayResponse) -> RouteResult<Value> {
    Ok(json!({
        "id": item.custom_id,
        "custom_id": item.custom_id,
        "response": {
            "status_code": response.status(),
            "body": response_json(response)?,
        },
    }))
}

fn response_error_record(
    custom_id: &str,
    status: Option<u16>,
    response: &GatewayResponse,
) -> RouteResult<Value> {
    Ok(json!({
        "custom_id": custom_id,
        "response": status.map(|status| json!({ "status_code": status })),
        "error": response_json(response)?["error"].clone(),
    }))
}

fn line_error_record(line_number: usize, error: OpenAiRouteError) -> RouteResult<Value> {
    let response = error.into_response();
    Ok(json!({
        "line": line_number,
        "error": response_json(&response)?["error"].clone(),
    }))
}

fn response_json(response: &GatewayResponse) -> RouteResult<Value> {
    serde_json::from_slice(response.body()).map_err(|err| {
        OpenAiRouteError::internal_message(format!("gateway response body is not JSON: {err}"))
    })
}

fn store_jsonl_file<S, C>(
    store: &mut S,
    ids: &mut BatchIdGenerators,
    clock: &mut C,
    batch_id: &str,
    suffix: &str,
    purpose: &str,
    records: &[Value],
) -> RouteResult<Option<String>>
where
    S: GatewayStateStore,
    C: WallClock,
{
    if records.is_empty() {
        return Ok(None);
    }
    let mut text = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    let bytes = text.into_bytes();
    let content_id =
        content_id_for_expr(&Expr::Bytes(bytes.clone())).map_err(OpenAiRouteError::internal)?;
    let file = GatewayFile::new(
        ids.file.next_id().map_err(OpenAiRouteError::internal)?,
        format!("{batch_id}-{suffix}.jsonl"),
        purpose,
        bytes.len() as u64,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
        GatewayFileStorageRef::memory(content_id),
    );
    let file_id = file.id().to_owned();
    store
        .put_file(file, bytes)
        .map_err(OpenAiRouteError::internal)?;
    Ok(Some(file_id))
}

fn batch_id_from_path(path: &str) -> Option<&str> {
    super::path::id_from_path(path, BATCH_RETRIEVAL_PREFIX)
}

fn cancel_batch_id_from_path(path: &str) -> Option<&str> {
    super::path::id_from_path_with_suffix(path, BATCH_RETRIEVAL_PREFIX, "/cancel")
}

fn batch_json(batch: &GatewayBatch) -> Value {
    json!({
        "id": batch.id(),
        "object": "batch",
        "endpoint": batch.endpoint(),
        "input_file_id": batch.input_file_id(),
        "status": batch.status().as_str(),
        "output_file_id": batch.output_file_id(),
        "error_file_id": batch.error_file_id(),
        "created_at": batch.created_at_ms(),
        "completed_at": batch.completed_at_ms(),
        "cancelled_at": batch.cancelled_at_ms(),
        "request_counts": counts_json(batch.request_counts()),
    })
}

fn counts_json(counts: GatewayBatchCounts) -> Value {
    json!({
        "total": counts.total(),
        "completed": counts.completed(),
        "failed": counts.failed(),
        "cancelled": counts.cancelled(),
    })
}
