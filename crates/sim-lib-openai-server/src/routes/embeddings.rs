use serde_json::{Map, Value, json};
use sim_cookbook::fnv1a64;
use sim_kernel::{Error, Expr, Symbol};

use crate::{
    clock::{SystemWallClock, WallClock},
    content_id::content_id_for_expr,
    objects::{GatewayRequest, GatewayResponse},
    plan::{check_plan, parse_plan, resolve_atom_address, shape::plan_parts},
    server::GatewayRouteState,
    storage::GatewayStore,
};

use super::{
    errors::OpenAiRouteError,
    execution_record::{EventInput, EventLog, RunPrologue, append_event, begin_run},
};

/// The embeddings engine shares the gateway execution-record substrate; its id
/// generators and execution outcome are the shared types under route-local
/// names.
pub use super::execution_record::{
    GatewayRunExecution as EmbeddingExecution, GatewayRunIdGenerators as EmbeddingIdGenerators,
};

/// Route path for the OpenAI-shaped `POST /v1/embeddings` endpoint.
pub const EMBEDDINGS_PATH: &str = "/v1/embeddings";
/// Model id of the built-in small fixed-dimension f64 embedding backend.
pub const TENSOR_F64_SMALL_EMBEDDING_MODEL: &str = "sim/embed/tensor-f64-small";

const TENSOR_F64_SMALL_DIMENSION: usize = 8;
const EMBEDDING_SCALE: u64 = 1_000_000;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

#[derive(Clone, Debug)]
struct EmbeddingModel {
    id: String,
    dimension: usize,
    runner: Symbol,
}

#[derive(Clone, Debug)]
struct EmbeddingUsage {
    prompt_tokens: u64,
    total_tokens: u64,
}

/// Handles `POST /v1/embeddings`, executing the request against the gateway
/// store and returning the OpenAI-shaped embeddings response.
pub fn handle_embeddings(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemWallClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = EmbeddingIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => execute_embedding_request(&mut *store, &mut ids, &mut clock, request)
            .response()
            .clone(),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Executes an embedding request end to end, recording request, run, and
/// events in the store and returning the [`EmbeddingExecution`] outcome.
///
/// Any failure is captured as an error response inside the returned execution
/// rather than propagated.
pub fn execute_embedding_request<S, C>(
    store: &mut S,
    ids: &mut EmbeddingIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> EmbeddingExecution
where
    S: GatewayStore,
    C: WallClock,
{
    match try_execute_embedding_request(store, ids, clock, request) {
        Ok(execution) => execution,
        Err(error) => EmbeddingExecution::error(error),
    }
}

fn try_execute_embedding_request<S, C>(
    store: &mut S,
    ids: &mut EmbeddingIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<EmbeddingExecution>
where
    S: GatewayStore,
    C: WallClock,
{
    let object = request_object(request.body())?;
    let model = required_string(&object, "model")?.to_owned();
    let inputs = embedding_inputs(&object)?;
    let record_execution = object.get("store").and_then(Value::as_bool).unwrap_or(true);

    let plan = parse_plan(&model).map_err(OpenAiRouteError::bad_model_from_error)?;
    check_plan(&plan).map_err(OpenAiRouteError::bad_model_from_error)?;
    let embedding_model = embedding_model_from_plan(&plan, &model)?;

    let RunPrologue {
        recorded_request,
        request_content_id,
        run_id,
        run_content_id,
    } = begin_run(store, ids, clock, request, None, record_execution)?;

    let embeddings = inputs
        .iter()
        .map(|input| embedding_for_text(&embedding_model.id, input, embedding_model.dimension))
        .collect::<Vec<_>>();
    let usage = embedding_usage(&inputs);

    let mut event_log = EventLog::default();
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(0, "request-start", recorded_request.to_expr()),
        record_execution,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(1, "plan-start", plan.clone()),
        record_execution,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(2, "model-start", Expr::String(embedding_model.id.clone())),
        record_execution,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(
            3,
            "embedding",
            embedding_event_expr(&embedding_model, inputs.len()),
        ),
        record_execution,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(4, "usage", usage_expr(&usage)),
        record_execution,
        &mut event_log,
    )?;

    let response_body = embedding_response_body(&embedding_model.id, &embeddings, &usage)?;
    let response = GatewayResponse::json(200, response_body);
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(
            5,
            "final",
            final_event_expr(&embedding_model, inputs.len(), &usage),
        ),
        record_execution,
        &mut event_log,
    )?;
    let response_content_id = if record_execution {
        let id = content_id_for_expr(&response.to_expr()).map_err(OpenAiRouteError::internal)?;
        store
            .put_response(id.clone(), response.clone())
            .map_err(OpenAiRouteError::internal)?;
        Some(id)
    } else {
        None
    };

    Ok(EmbeddingExecution {
        response,
        request_content_id: Some(request_content_id),
        run_content_id: Some(run_content_id),
        event_content_ids: event_log.content_ids,
        events: event_log.events,
        response_id: None,
        response_created_at_ms: None,
        response_content_id,
    })
}

use crate::routes::request_json::{request_object, required_string};

fn embedding_inputs(object: &Map<String, Value>) -> RouteResult<Vec<String>> {
    match object.get("input") {
        Some(Value::String(input)) => Ok(vec![input.clone()]),
        Some(Value::Array(inputs)) => inputs
            .iter()
            .map(|input| match input {
                Value::String(text) => Ok(text.clone()),
                _ => Err(OpenAiRouteError::bad_request(
                    "embeddings input list must contain only strings",
                    Some("input"),
                    "invalid_input",
                )),
            })
            .collect(),
        Some(_) => Err(OpenAiRouteError::bad_request(
            "embeddings input must be a string or list of strings",
            Some("input"),
            "invalid_input",
        )),
        None => Err(OpenAiRouteError::missing_required("input")),
    }
}

fn embedding_model_from_plan(plan: &Expr, model: &str) -> RouteResult<EmbeddingModel> {
    let (name, args) = plan_parts(plan).map_err(OpenAiRouteError::bad_model_from_error)?;
    if name != "atom" {
        return Err(OpenAiRouteError::bad_request(
            "embeddings model must be a plan atom",
            Some("model"),
            "invalid_model",
        ));
    }
    let [Expr::String(address)] = args else {
        return Err(OpenAiRouteError::bad_model_from_error(Error::Eval(
            "plan/atom expects one address".to_owned(),
        )));
    };
    let descriptor =
        resolve_atom_address(address).map_err(|err| OpenAiRouteError::model(err, model))?;
    if !descriptor.address.starts_with("sim/embed/") {
        return Err(model_not_found(model));
    }
    let dimension = match descriptor.address.as_str() {
        TENSOR_F64_SMALL_EMBEDDING_MODEL => TENSOR_F64_SMALL_DIMENSION,
        _ => return Err(model_not_found(model)),
    };
    Ok(EmbeddingModel {
        id: descriptor.address,
        dimension,
        runner: descriptor.runner,
    })
}

fn model_not_found(model: &str) -> OpenAiRouteError {
    OpenAiRouteError::model(Error::Eval(format!("model_not_found: {model}")), model)
}

fn embedding_for_text(model: &str, input: &str, dimension: usize) -> Vec<f64> {
    (0..dimension)
        .map(|index| hash_to_unit(stable_embedding_hash(model, input, index)))
        .collect()
}

fn stable_embedding_hash(model: &str, input: &str, dimension_index: usize) -> u64 {
    let mut bytes =
        Vec::with_capacity(model.len() + input.len() + 2 + std::mem::size_of::<usize>());
    bytes.extend_from_slice(model.as_bytes());
    bytes.push(0xff);
    bytes.extend_from_slice(input.as_bytes());
    bytes.push(0xfe);
    bytes.extend_from_slice(&dimension_index.to_le_bytes());
    fnv1a64(&bytes)
}
fn hash_to_unit(hash: u64) -> f64 {
    let bucket = hash % (EMBEDDING_SCALE * 2 + 1);
    (bucket as f64 / EMBEDDING_SCALE as f64) - 1.0
}
fn embedding_usage(inputs: &[String]) -> EmbeddingUsage {
    let prompt_tokens = inputs
        .iter()
        .map(|input| input.split_whitespace().count() as u64)
        .sum();
    EmbeddingUsage {
        prompt_tokens,
        total_tokens: prompt_tokens,
    }
}

fn embedding_response_body(
    model: &str,
    embeddings: &[Vec<f64>],
    usage: &EmbeddingUsage,
) -> RouteResult<Vec<u8>> {
    Ok(crate::objects::canonical_json_bytes(json!({
        "object": "list",
        "data": embeddings
            .iter()
            .enumerate()
            .map(|(index, embedding)| {
                json!({
                    "object": "embedding",
                    "embedding": embedding,
                    "index": index,
                })
            })
            .collect::<Vec<_>>(),
        "model": model,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "total_tokens": usage.total_tokens,
        },
    })))
}

fn embedding_event_expr(model: &EmbeddingModel, input_count: usize) -> Expr {
    Expr::Map(vec![
        field("model", Expr::String(model.id.clone())),
        field("runner", Expr::Symbol(model.runner.clone())),
        field("input-count", Expr::String(input_count.to_string())),
        field("dimension", Expr::String(model.dimension.to_string())),
    ])
}

// Token counts are Expr::String by design here, consistent with the sibling
// embeddings event fields (input-count, dimension). Numeric token fields would
// make this record internally inconsistent.
fn usage_expr(usage: &EmbeddingUsage) -> Expr {
    Expr::Map(vec![
        field(
            "prompt-tokens",
            Expr::String(usage.prompt_tokens.to_string()),
        ),
        field("total-tokens", Expr::String(usage.total_tokens.to_string())),
    ])
}

fn final_event_expr(model: &EmbeddingModel, input_count: usize, usage: &EmbeddingUsage) -> Expr {
    Expr::Map(vec![
        field("model", Expr::String(model.id.clone())),
        field("object", Expr::String("list".to_owned())),
        field("input-count", Expr::String(input_count.to_string())),
        field("dimension", Expr::String(model.dimension.to_string())),
        field("usage", usage_expr(usage)),
    ])
}

use sim_value::build::entry as field;
