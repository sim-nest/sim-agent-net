//! Golden snapshots pinning the gateway execution-record wire (OVERLAP9.03).
//!
//! These snapshots freeze the observable output of the three parallel record
//! engines -- `/v1/responses`, `/v1/embeddings`, and one run_record route (audio
//! transcription) -- BEFORE the OVERLAP9.04 substrate collapse. They capture the
//! stored request/run/event/response content-ids, the per-event id/kind/sequence
//! /payload, the final wire payload, and (for the object-retrievable responses
//! path) the `/events`, `/sim`, and replay renderings. OVERLAP9.04 must keep every
//! one of these byte-identical: if a golden drifts, that part of the collapse is
//! wrong.
//!
//! There is no `insta` dependency in this crate, so the goldens are plain JSON
//! files under `src/tests/goldens/`. Run the suite with `SIM_BLESS_GOLDENS=1` to
//! (re)write them; the committed files are the contract.

use std::path::PathBuf;

use serde_json::{Value, json};
use sim_kernel::{CapabilitySet, ContentId};

use crate::{
    AUDIO_TRANSCRIPTIONS_PATH, DeterministicGatewayClock, EMBEDDINGS_PATH, EmbeddingIdGenerators,
    GatewayEvent, GatewayRequest, GatewayResponse, GatewayResponseObjectStore, GatewayRouteState,
    GatewayStore, MemoryGatewayStore, OpenAiKeyTable, RESPONSES_PATH, ResponseIdGenerators,
    SIM_REPLAY_PATH, TENSOR_F64_SMALL_EMBEDDING_MODEL, configure_routes_with_state, content_id_hex,
    execute_embedding_request, execute_response_request, openai_gateway_admin_capability,
    response_events, response_sim,
    routes::{audio::execute_audio_transcription_request, run_record::RouteRunIdGenerators},
};

const GOLDEN_ADMIN_SECRET: &str = "sk-execution-record-admin";

// --- Responses (`/v1/responses`): object-retrievable via put_response_object ---

#[test]
fn responses_execution_record_store_true_golden() {
    assert_golden(
        "responses_execution_record_store_true",
        &responses_golden(true),
    );
}

#[test]
fn responses_execution_record_store_false_golden() {
    assert_golden(
        "responses_execution_record_store_false",
        &responses_golden(false),
    );
}

fn responses_golden(store: bool) -> Value {
    let mut cx = super::cx();
    let mut gateway_store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicGatewayClock::new(1_000, 10);
    let request = json_request(
        RESPONSES_PATH,
        &format!(r#"{{"model":"fixture/echo","input":"golden input","store":{store}}}"#),
    );

    let execution =
        execute_response_request(&mut cx, &mut gateway_store, &mut ids, &mut clock, &request);

    let mut value = execution_base(
        store,
        execution.request_content_id(),
        execution.run_content_id(),
        execution.event_content_ids(),
        execution.events(),
        execution.response_content_id(),
        execution.response(),
    );
    value["response_id"] = json!(execution.response_id());

    // Object-vs-bare retrieval: responses are retrievable by their public id via
    // put_response_object, and that unlocks the /events, /sim, and replay views.
    let response_id = execution.response_id().map(str::to_owned);
    let object_retrievable = response_id
        .as_deref()
        .map(|id| gateway_store.response_object(id).is_some())
        .unwrap_or(false);
    value["object_retrievable_by_id"] = json!(object_retrievable);

    if object_retrievable {
        let response_id = response_id.clone().unwrap();
        value["events_view"] = response_body_json(&response_events(&gateway_store, &response_id));
        value["sim_view"] = response_body_json(&response_sim(&gateway_store, &response_id));

        let key_table = OpenAiKeyTable::new().unwrap();
        key_table
            .add_secret(
                GOLDEN_ADMIN_SECRET,
                CapabilitySet::new().grant(openai_gateway_admin_capability()),
            )
            .unwrap();
        let state = GatewayRouteState::new(gateway_store).with_keys(key_table);
        let routes = configure_routes_with_state(state);
        let replay = routes.handle(&authorized_json_request(
            GOLDEN_ADMIN_SECRET,
            SIM_REPLAY_PATH,
            &json!({ "response_id": response_id }).to_string(),
        ));
        value["replay_view"] = response_body_json(&replay);
    }

    value
}

// --- Embeddings (`/v1/embeddings`): bare via put_response, store defaults true ---

#[test]
fn embeddings_execution_record_store_true_golden() {
    assert_golden(
        "embeddings_execution_record_store_true",
        &embeddings_golden(true),
    );
}

#[test]
fn embeddings_execution_record_store_false_golden() {
    assert_golden(
        "embeddings_execution_record_store_false",
        &embeddings_golden(false),
    );
}

fn embeddings_golden(store: bool) -> Value {
    let mut gateway_store = MemoryGatewayStore::new();
    let mut ids = EmbeddingIdGenerators::deterministic(7);
    let mut clock = DeterministicGatewayClock::new(2_000, 20);
    let request = json_request(
        EMBEDDINGS_PATH,
        &format!(
            r#"{{"model":"{TENSOR_F64_SMALL_EMBEDDING_MODEL}","input":["hello","wide world"],"store":{store}}}"#
        ),
    );

    let execution = execute_embedding_request(&mut gateway_store, &mut ids, &mut clock, &request);

    let mut value = execution_base(
        store,
        execution.request_content_id(),
        execution.run_content_id(),
        execution.event_content_ids(),
        execution.events(),
        execution.response_content_id(),
        execution.response(),
    );
    bare_retrieval(&mut value, &gateway_store, execution.response_content_id());
    value
}

// --- run_record (`/v1/audio/transcriptions`): bare via put_response ---

#[test]
fn run_record_audio_execution_record_store_true_golden() {
    assert_golden(
        "run_record_audio_execution_record_store_true",
        &audio_golden(true),
    );
}

#[test]
fn run_record_audio_execution_record_store_false_golden() {
    assert_golden(
        "run_record_audio_execution_record_store_false",
        &audio_golden(false),
    );
}

fn audio_golden(store: bool) -> Value {
    let mut gateway_store = MemoryGatewayStore::new();
    let mut ids = RouteRunIdGenerators::deterministic(11);
    let mut clock = DeterministicGatewayClock::new(3_000, 30);
    let request = json_request(
        AUDIO_TRANSCRIPTIONS_PATH,
        &format!(r#"{{"input":"golden   SIM audio","store":{store}}}"#),
    );

    let execution =
        execute_audio_transcription_request(&mut gateway_store, &mut ids, &mut clock, &request);

    let mut value = execution_base(
        store,
        execution.request_content_id(),
        execution.run_content_id(),
        execution.event_content_ids(),
        execution.events(),
        execution.response_content_id(),
        execution.response(),
    );
    bare_retrieval(&mut value, &gateway_store, execution.response_content_id());
    value
}

// --- shared renderers -------------------------------------------------------

fn execution_base(
    store: bool,
    request_content_id: Option<&ContentId>,
    run_content_id: Option<&ContentId>,
    event_content_ids: &[ContentId],
    events: &[GatewayEvent],
    response_content_id: Option<&ContentId>,
    response: &GatewayResponse,
) -> Value {
    json!({
        "store": store,
        "request_content_id": request_content_id.map(content_id_hex),
        "run_content_id": run_content_id.map(content_id_hex),
        "event_content_ids": event_content_ids.iter().map(content_id_hex).collect::<Vec<_>>(),
        "events": events.iter().map(event_value).collect::<Vec<_>>(),
        "response_content_id": response_content_id.map(content_id_hex),
        "final_response": response_value(response),
    })
}

fn bare_retrieval(
    value: &mut Value,
    store: &MemoryGatewayStore,
    response_content_id: Option<&ContentId>,
) {
    // Bare storage: the response lives under its content id via put_response, and
    // there is no put_response_object entry -- so no /events, /sim, or replay view.
    value["bare_retrievable_by_content_id"] =
        json!(response_content_id.map(|id| store.response(id).is_some()));
    value["object_retrievable_by_id"] = json!(false);
}

fn event_value(event: &GatewayEvent) -> Value {
    json!({
        "id": event.id(),
        "run_id": event.run_id(),
        "sequence": event.sequence(),
        "kind": event.kind().name.as_ref(),
        "created_at_ms": event.created_at_ms(),
        "payload_debug": format!("{:?}", event.payload()),
    })
}

fn response_value(response: &GatewayResponse) -> Value {
    json!({
        "status": response.status(),
        "content_type": response.header("Content-Type"),
        "body": response_body_json(response),
    })
}

fn response_body_json(response: &GatewayResponse) -> Value {
    match serde_json::from_slice::<Value>(response.body()) {
        Ok(json) => json,
        Err(_) => Value::String(format!("<non-json body, {} bytes>", response.body().len())),
    }
}

fn json_request(path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn authorized_json_request(secret: &str, path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        path,
        vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), format!("Bearer {secret}")),
        ],
        body.as_bytes().to_vec(),
    )
}

// --- golden file harness ----------------------------------------------------

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tests/goldens")
        .join(format!("{name}.json"))
}

fn assert_golden(name: &str, value: &Value) {
    let path = golden_path(name);
    let actual = format!("{}\n", serde_json::to_string_pretty(value).unwrap());

    if std::env::var_os("SIM_BLESS_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "missing golden {}: {err}; regenerate with SIM_BLESS_GOLDENS=1",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "gateway execution-record golden {name} drifted -- OVERLAP9.04 must keep it byte-identical"
    );
}
