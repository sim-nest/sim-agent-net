use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use sim_kernel::{CapabilityName, CapabilitySet, Symbol};
use sim_lib_agent_runner_http::HttpRunner;
use sim_lib_openai_server::{
    GatewayRequest, GatewayResponse, GatewayRouteState, MODELS_PATH, MemoryGatewayStore,
    OpenAiKeyTable, OpenAiRunnerRegistry, RESPONSES_PATH, configure_routes_with_state,
};
use sim_transport_ports::model::ScriptedStreamPort;

static TRANSPORT_TEST: Mutex<()> = Mutex::new(());

#[test]
fn openai_compatible_runner_reaches_mock_provider_through_gateway() {
    let _guard = TRANSPORT_TEST.lock().unwrap();
    let body = r#"{"id":"chatcmpl-mock","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"mock ok"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let transport = Arc::new(ScriptedStreamPort::new([response.into_bytes()]));
    sim_transport_ports::bind_services(transport.services()).unwrap();
    let routes =
        configure_routes_with_state(route_state("http://provider.test:8080/v1".to_owned()));

    let models = routes.handle(&GatewayRequest::get(MODELS_PATH));
    let models_json = response_json(&models);
    assert!(
        models_json["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "openai/gpt-4o-mini"
                && model["owned_by"] == "openai-compatible")
    );

    let response = routes.handle(&GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"openai/gpt-4o-mini","input":"hello outbound","store":true}"#.to_vec(),
    ));
    let json = response_json(&response);
    let request = String::from_utf8(transport.requests().into_iter().next().unwrap()).unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(json["object"], "response");
    assert_eq!(json["output_text"], "mock ok");
    assert_eq!(json["usage"]["total_tokens"], 5);
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    let provider_body: Value = serde_json::from_slice(provider_body(&request).as_bytes()).unwrap();
    assert_eq!(provider_body["model"], "gpt-4o-mini");
    assert!(provider_body.to_string().contains("hello outbound"));
}

#[test]
fn unavailable_remote_runner_returns_structured_error_response() {
    let _guard = TRANSPORT_TEST.lock().unwrap();
    let transport = Arc::new(ScriptedStreamPort::new([Vec::new()]));
    sim_transport_ports::bind_services(transport.services()).unwrap();
    let routes =
        configure_routes_with_state(route_state("http://provider.test:8080/v1".to_owned()));

    let response = routes.handle(&GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"openai/gpt-4o-mini","input":"provider is down"}"#.to_vec(),
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(json["object"], "response");
    assert_eq!(json["status"], "completed");
    assert!(
        json["output_text"]
            .as_str()
            .unwrap()
            .contains("runner/openai-compatible")
    );
}

#[cfg(not(feature = "tls"))]
#[test]
fn https_runner_is_rejected_without_tls_feature() {
    let routes = configure_routes_with_state(route_state("https://provider.invalid/v1".to_owned()));

    let response = routes.handle(&GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"openai/gpt-4o-mini","input":"tls check"}"#.to_vec(),
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert!(
        json["output_text"]
            .as_str()
            .unwrap()
            .contains("https endpoints require")
    );
}

fn route_state(endpoint: String) -> GatewayRouteState {
    let runner = HttpRunner::new_openai_compatible(
        Symbol::qualified("runner", "openai-compatible"),
        "gpt-4o-mini",
        endpoint,
        "CARGO_MANIFEST_DIR",
        Symbol::qualified("codec", "openai"),
        Duration::from_secs(2),
        false,
        true,
        64 * 1024,
    );
    let registry = OpenAiRunnerRegistry::new().with_runner("openai/gpt-4o-mini", Arc::new(runner));
    GatewayRouteState::new(MemoryGatewayStore::new())
        .with_runners(registry)
        .with_keys(OpenAiKeyTable::with_anonymous(runner_capabilities()).unwrap())
}

fn runner_capabilities() -> CapabilitySet {
    CapabilitySet::new()
        .grant(CapabilityName::new("ai-runner"))
        .grant(CapabilityName::new("ai-runner-network"))
        .grant(CapabilityName::new("ai-runner-secret"))
}

fn provider_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap()
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}
