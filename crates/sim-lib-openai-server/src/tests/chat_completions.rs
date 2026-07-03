use serde_json::Value;
use sim_kernel::ContentId;

use crate::{
    CHAT_COMPLETIONS_PATH, DeterministicGatewayClock, GatewayEvent, GatewayRequest, GatewayStore,
    MemoryGatewayStore, ResponseIdGenerators, configure_routes, execute_chat_completion_request,
    execute_response_request, gateway_event_data_from_packet, gateway_event_data_packets,
    routes::chat_completions::chat_completion_runtime_request,
};

#[test]
fn chat_completions_route_fixture_echo_returns_chat_completion() {
    let response = configure_routes().handle(&chat_request(
        r#"{"model":"fixture/echo","messages":[{"role":"system","content":"brief"},{"role":"user","content":"hello"}]}"#,
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(json["object"], "chat.completion");
    assert!(json["id"].as_str().unwrap().starts_with("chatcmpl_"));
    assert_eq!(json["model"], "fixture/echo");
    assert_eq!(json["choices"][0]["message"]["role"], "assistant");
    assert_eq!(json["choices"][0]["message"]["content"], "hello brief");
    assert_eq!(json["choices"][0]["finish_reason"], "stop");
    assert_eq!(json["usage"]["total_tokens"], 4);
}

#[test]
fn chat_completions_route_streams_sse_chunks() {
    let response = configure_routes().handle(&chat_request(
        r#"{"model":"fixture/echo","messages":[{"role":"system","content":"brief"},{"role":"user","content":"hello"}],"stream":true}"#,
    ));
    let chunks = super::sse_json_chunks(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("text/event-stream"));
    assert!(super::sse_ends_with_done(&response));
    assert_eq!(chunks[0]["object"], "chat.completion.chunk");
    assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
    assert_eq!(chat_stream_text(&chunks), "hello brief");
    let final_chunk = chunks
        .iter()
        .find(|chunk| chunk["choices"][0]["finish_reason"] == "stop")
        .unwrap();
    assert_eq!(final_chunk["choices"][0]["delta"], serde_json::json!({}));
}

#[test]
fn chat_completions_route_rejects_unsupported_roles() {
    let response = configure_routes().handle(&chat_request(
        r#"{"model":"fixture/echo","messages":[{"role":"tool","content":"x"},{"role":"user","content":"hello"}]}"#,
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 400);
    assert_eq!(json["error"]["param"], "messages");
    assert_eq!(json["error"]["code"], "unsupported_role");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unsupported chat message role")
    );
}

#[test]
fn chat_completions_and_responses_share_runtime_event_log() {
    let chat = chat_request(
        r#"{"model":"fixture/echo","messages":[{"role":"system","content":"brief"},{"role":"user","content":"hello"}],"store":true}"#,
    );
    let runtime_request = chat_completion_runtime_request(&chat).unwrap();

    let mut chat_cx = super::cx();
    let mut chat_store = MemoryGatewayStore::new();
    let mut chat_ids = ResponseIdGenerators::deterministic(7);
    let mut chat_clock = DeterministicGatewayClock::new(1_000, 10);
    let chat_execution = execute_chat_completion_request(
        &mut chat_cx,
        &mut chat_store,
        &mut chat_ids,
        &mut chat_clock,
        &chat,
    );

    let mut response_cx = super::cx();
    let mut response_store = MemoryGatewayStore::new();
    let mut response_ids = ResponseIdGenerators::deterministic(7);
    let mut response_clock = DeterministicGatewayClock::new(1_000, 10);
    let response_execution = execute_response_request(
        &mut response_cx,
        &mut response_store,
        &mut response_ids,
        &mut response_clock,
        &runtime_request,
    );

    assert_eq!(chat_execution.response().status(), 200);
    assert_eq!(response_execution.response().status(), 200);
    assert_eq!(
        response_json(chat_execution.response())["object"],
        "chat.completion"
    );
    assert_eq!(
        response_json(response_execution.response())["object"],
        "response"
    );

    let chat_runtime = chat_execution.runtime().unwrap();
    assert_eq!(
        stored_events(&chat_store, chat_runtime.event_content_ids()),
        stored_events(&response_store, response_execution.event_content_ids())
    );
}

#[test]
fn streaming_chat_completion_data_packets_reconstruct_event_log() {
    let chat = chat_request(
        r#"{"model":"fixture/echo","messages":[{"role":"system","content":"brief"},{"role":"user","content":"hello"}],"store":true,"stream":true}"#,
    );
    let mut cx = super::cx();
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(13);
    let mut clock = DeterministicGatewayClock::new(3_000, 30);

    let execution =
        execute_chat_completion_request(&mut cx, &mut store, &mut ids, &mut clock, &chat);
    let runtime = execution.runtime().unwrap();
    let packets = gateway_event_data_packets(runtime.events());
    let data = packets
        .iter()
        .map(gateway_event_data_from_packet)
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();

    assert_eq!(
        execution.response().header("Content-Type"),
        Some("text/event-stream")
    );
    assert_eq!(
        chat_stream_text(&super::sse_json_chunks(execution.response())),
        "hello brief"
    );
    assert_eq!(data.len(), runtime.events().len());
    for (data, event) in data.iter().zip(runtime.events()) {
        assert_eq!(data.sequence(), event.sequence());
        assert_eq!(data.kind(), event.kind());
        assert_eq!(data.payload(), event.payload());
    }
}

fn chat_request(body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        CHAT_COMPLETIONS_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn stored_events(store: &MemoryGatewayStore, ids: &[ContentId]) -> Vec<GatewayEvent> {
    ids.iter().map(|id| store.event(id).unwrap()).collect()
}

fn response_json(response: &crate::GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn chat_stream_text(chunks: &[Value]) -> String {
    chunks
        .iter()
        .filter_map(|chunk| chunk["choices"].as_array())
        .filter_map(|choices| choices.first())
        .filter_map(|choice| choice["delta"]["content"].as_str())
        .collect()
}
