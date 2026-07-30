use serde_json::Value;

use crate::{
    AUDIO_SPEECH_PATH, AUDIO_TRANSCRIPTIONS_PATH, DeterministicWallClock, GatewayRequest,
    GatewayResponse, GatewayRouteState, GatewayStateStore, GatewayStore, GatewayVectorStore,
    GatewayVectorStoreItem, IMAGES_GENERATIONS_PATH, MemoryGatewayStore, TableGatewayStore,
    VECTOR_STORE_SEARCH_ROUTE, VECTOR_STORES_PATH, configure_routes, configure_routes_with_state,
    routes::{
        audio::execute_audio_transcription_request, run_record::RouteRunIdGenerators,
        vector_stores::VECTOR_STORE_SEARCH_SUFFIX,
    },
};

#[test]
fn phase21_routes_are_registered() {
    let paths = configure_routes().paths();

    for path in [
        AUDIO_TRANSCRIPTIONS_PATH,
        AUDIO_SPEECH_PATH,
        IMAGES_GENERATIONS_PATH,
        VECTOR_STORES_PATH,
        VECTOR_STORE_SEARCH_ROUTE,
    ] {
        assert!(paths.contains(&path), "missing route {path}");
    }
}

#[test]
fn audio_transcription_records_replayable_fixture_run() {
    let mut store = MemoryGatewayStore::new();
    let mut ids = RouteRunIdGenerators::deterministic(7);
    let mut clock = DeterministicWallClock::new(1_000, 10);
    let request = json_request(
        AUDIO_TRANSCRIPTIONS_PATH,
        r#"{"input":"hello   SIM audio","store":true}"#,
    );

    let execution = execute_audio_transcription_request(&mut store, &mut ids, &mut clock, &request);
    let json = response_json(execution.response());

    assert_eq!(execution.response().status(), 200);
    assert_eq!(json["text"], "transcribed: hello SIM audio");
    assert!(
        store
            .request(execution.request_content_id().unwrap())
            .is_some()
    );
    assert!(store.run(execution.run_content_id().unwrap()).is_some());
    assert_eq!(execution.event_content_ids().len(), 4);
    assert_eq!(execution.events().len(), 4);
    assert!(
        store
            .response(execution.response_content_id().unwrap())
            .is_some()
    );
}

#[test]
fn audio_and_image_fixtures_are_deterministic() {
    let routes = configure_routes();
    let transcription = json_request(
        AUDIO_TRANSCRIPTIONS_PATH,
        r#"{"input":"deterministic audio"}"#,
    );
    let speech = json_request(
        AUDIO_SPEECH_PATH,
        r#"{"input":"hello","voice":"verse","response_format":"wav"}"#,
    );
    let image = json_request(IMAGES_GENERATIONS_PATH, r#"{"prompt":"unit square","n":2}"#);

    let transcription_first = routes.handle(&transcription);
    let transcription_second = routes.handle(&transcription);
    let speech_first = routes.handle(&speech);
    let speech_second = routes.handle(&speech);
    let image_first = routes.handle(&image);
    let image_second = routes.handle(&image);

    assert_eq!(transcription_first.status(), 200);
    assert_eq!(transcription_first.body(), transcription_second.body());
    assert_eq!(speech_first.status(), 200);
    assert_eq!(speech_first.header("Content-Type"), Some("audio/wav"));
    assert_eq!(speech_first.body(), speech_second.body());
    assert_eq!(image_first.status(), 200);
    assert_eq!(image_first.body(), image_second.body());
    assert_eq!(
        response_json(&image_first)["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn vector_store_search_returns_deterministic_nearest_items() {
    let state = GatewayRouteState::memory();
    let routes = configure_routes_with_state(state.clone());
    let create = routes.handle(&json_request(
        VECTOR_STORES_PATH,
        r#"{"name":"fixture","items":["alpha","beta","gamma"]}"#,
    ));
    let create_json = response_json(&create);
    let vector_store_id = create_json["id"].as_str().unwrap();
    let search_path = format!(
        "/v1/vector_stores/{vector_store_id}{}",
        VECTOR_STORE_SEARCH_SUFFIX
    );

    assert_eq!(create.status(), 200);
    assert!(
        state
            .store()
            .lock()
            .unwrap()
            .vector_store(vector_store_id)
            .is_some()
    );

    let request = json_request(&search_path, r#"{"query":"alpha","limit":2}"#);
    let first = routes.handle(&request);
    let second = routes.handle(&request);
    let json = response_json(&first);

    assert_eq!(first.status(), 200);
    assert_eq!(first.body(), second.body());
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
    assert_eq!(json["data"][0]["text"], "alpha");
    assert_eq!(json["data"][0]["score"], 1.0);
}

#[test]
fn vector_store_objects_round_trip_through_table_store() {
    let mut store = TableGatewayStore::new().unwrap();
    let vector_store = GatewayVectorStore::new(
        "vs_table",
        "table-backed",
        1,
        vec![GatewayVectorStoreItem::new(
            "vsi_table",
            "alpha",
            vec![1.0, 0.0],
        )],
    );

    store.put_vector_store(vector_store.clone()).unwrap();

    assert_eq!(store.vector_store("vs_table"), Some(vector_store));
}

fn json_request(path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}
