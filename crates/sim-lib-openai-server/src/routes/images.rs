use serde_json::json;
use sim_cookbook::fnv1a64_hex;
use sim_kernel::Expr;

use crate::{
    clock::{GatewayClock, SystemGatewayClock},
    objects::{GatewayRequest, GatewayResponse},
    routes::{
        request_json::{
            optional_string, optional_u64, record_execution, request_object, required_string,
        },
        run_record::{
            RouteEventInput, RouteRunExecution, RouteRunIdGenerators, RouteRunRecord, field,
            record_route_execution,
        },
    },
    server::GatewayRouteState,
    storage::GatewayStore,
};

use super::errors::OpenAiRouteError;

/// Route path for image generation (`POST /v1/images/generations`).
pub const IMAGES_GENERATIONS_PATH: &str = "/v1/images/generations";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Handles `POST /v1/images/generations`, returning deterministic fixture image
/// references for the requested JSON prompt and count.
pub fn handle_image_generations(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = RouteRunIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => {
            execute_image_generation_request(&mut *store, &mut ids, &mut clock, request)
                .response()
                .clone()
        }
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

pub(crate) fn execute_image_generation_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteRunExecution
where
    S: GatewayStore,
    C: GatewayClock,
{
    match try_execute_image_generation_request(store, ids, clock, request) {
        Ok(execution) => execution,
        Err(error) => RouteRunExecution::error(error),
    }
}

fn try_execute_image_generation_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let model = optional_string(&object, "model", "sim/image/fixture");
    let prompt = required_string(&object, "prompt")?;
    let count = optional_u64(&object, "n", 1)?;
    if !(1..=10).contains(&count) {
        return Err(OpenAiRouteError::bad_request(
            "n must be between 1 and 10",
            Some("n"),
            "invalid_request",
        ));
    }

    let data = (0..count)
        .map(|index| {
            json!({
                "url": format!("sim://image/{}", stable_image_id(model, prompt, index)),
                "revised_prompt": prompt,
                "index": index,
            })
        })
        .collect::<Vec<_>>();
    let response = GatewayResponse::json(
        200,
        json!({
            "object": "image.generation",
            "created": 0,
            "model": model,
            "data": data,
        })
        .to_string()
        .into_bytes(),
    );
    record_route_execution(
        store,
        ids,
        clock,
        request,
        RouteRunRecord::new(
            response,
            IMAGES_GENERATIONS_PATH,
            vec![RouteEventInput::new(
                "image-generation",
                image_event_payload(model, prompt, count),
            )],
            record_execution(&object),
        ),
    )
}

fn stable_image_id(model: &str, prompt: &str, index: u64) -> String {
    let mut bytes = Vec::with_capacity(model.len() + prompt.len() + 10);
    bytes.extend_from_slice(model.as_bytes());
    bytes.push(0xff);
    bytes.extend_from_slice(prompt.as_bytes());
    bytes.push(0xfe);
    bytes.extend_from_slice(&index.to_le_bytes());
    fnv1a64_hex(&bytes)
}

fn image_event_payload(model: &str, prompt: &str, count: u64) -> Expr {
    Expr::Map(vec![
        field("model", Expr::String(model.to_owned())),
        field("prompt", Expr::String(prompt.to_owned())),
        field("count", Expr::String(count.to_string())),
    ])
}
