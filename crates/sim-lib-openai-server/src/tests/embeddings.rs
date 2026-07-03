use serde_json::Value;
use sim_kernel::{ContentId, Expr};

use crate::{
    DeterministicGatewayClock, EMBEDDINGS_PATH, EmbeddingIdGenerators, GatewayEvent,
    GatewayRequest, GatewayStore, MemoryGatewayStore, TENSOR_F64_SMALL_EMBEDDING_MODEL,
    configure_routes, execute_embedding_request,
};

#[test]
fn embeddings_route_single_input_returns_openai_embedding_object() {
    let response = configure_routes().handle(&embeddings_request(&format!(
        r#"{{"model":"{TENSOR_F64_SMALL_EMBEDDING_MODEL}","input":"hello"}}"#
    )));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(json["object"], "list");
    assert_eq!(json["model"], TENSOR_F64_SMALL_EMBEDDING_MODEL);
    assert_eq!(json["data"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"][0]["object"], "embedding");
    assert_eq!(json["data"][0]["index"], 0);
    assert_eq!(embedding_len(&json, 0), 8);
    assert_eq!(json["usage"]["prompt_tokens"], 1);
    assert_eq!(json["usage"]["total_tokens"], 1);
}

#[test]
fn embeddings_route_batch_input_is_deterministic_with_stable_dimension() {
    let request = embeddings_request(&format!(
        r#"{{"model":"{TENSOR_F64_SMALL_EMBEDDING_MODEL}","input":["hello","world"]}}"#
    ));

    let first = configure_routes().handle(&request);
    let second = configure_routes().handle(&request);
    let first_json = response_json(&first);
    let second_json = response_json(&second);

    assert_eq!(first.status(), 200);
    assert_eq!(first.body(), second.body());
    assert_eq!(first_json["data"].as_array().unwrap().len(), 2);
    assert_eq!(embedding_len(&first_json, 0), 8);
    assert_eq!(embedding_len(&first_json, 1), 8);
    assert_ne!(
        first_json["data"][0]["embedding"],
        first_json["data"][1]["embedding"]
    );
    assert_eq!(first_json, second_json);
}

#[test]
fn embedding_execution_records_run_usage_and_response_when_stored() {
    let mut store = MemoryGatewayStore::new();
    let mut ids = EmbeddingIdGenerators::deterministic(7);
    let mut clock = DeterministicGatewayClock::new(1_000, 10);
    let request = embeddings_request(&format!(
        r#"{{"model":"{TENSOR_F64_SMALL_EMBEDDING_MODEL}","input":["hello","wide world"],"store":true}}"#
    ));

    let execution = execute_embedding_request(&mut store, &mut ids, &mut clock, &request);

    assert_eq!(execution.response().status(), 200);
    let request_content_id = execution.request_content_id().unwrap();
    let run_content_id = execution.run_content_id().unwrap();
    assert!(store.request(request_content_id).is_some());
    assert!(store.run(run_content_id).is_some());
    assert_eq!(execution.event_content_ids().len(), 6);
    assert_eq!(
        stored_events(&store, execution.event_content_ids()),
        execution.events()
    );
    let usage_event = execution
        .events()
        .iter()
        .find(|event| event.kind().name.as_ref() == "usage")
        .unwrap();
    assert_eq!(
        expr_field(usage_event.payload(), "total-tokens"),
        &Expr::String("3".to_owned())
    );
    let response_content_id = execution.response_content_id().unwrap();
    assert_eq!(
        store.response(response_content_id),
        Some(execution.response().clone())
    );
}

#[test]
fn embeddings_route_rejects_non_embedding_model() {
    let response = configure_routes().handle(&embeddings_request(
        r#"{"model":"fixture/echo","input":"hello"}"#,
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 404);
    assert_eq!(json["error"]["param"], "model");
    assert_eq!(json["error"]["code"], "model_not_found");
}

fn embeddings_request(body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        EMBEDDINGS_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &crate::GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn embedding_len(json: &Value, index: usize) -> usize {
    json["data"][index]["embedding"].as_array().unwrap().len()
}

fn stored_events(store: &MemoryGatewayStore, ids: &[ContentId]) -> Vec<GatewayEvent> {
    ids.iter().map(|id| store.event(id).unwrap()).collect()
}

fn expr_field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map expression, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
                Some(value)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected field {name} in {expr:?}"))
}
