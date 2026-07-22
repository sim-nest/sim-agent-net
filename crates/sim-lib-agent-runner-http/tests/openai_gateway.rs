use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::Value;
use sim_kernel::{CapabilityName, CapabilitySet, Symbol};
use sim_lib_agent_runner_http::HttpRunner;
use sim_lib_openai_server::{
    GatewayRequest, GatewayResponse, GatewayRouteState, MODELS_PATH, MemoryGatewayStore,
    OpenAiKeyTable, OpenAiRunnerRegistry, RESPONSES_PATH, configure_routes_with_state,
};

#[test]
fn openai_compatible_runner_reaches_mock_provider_through_gateway() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let server = spawn_openai_mock(listener);
    let routes = configure_routes_with_state(route_state(format!("http://127.0.0.1:{port}/v1")));

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
    let request = server.join().unwrap();

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
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let server = spawn_closing_server(listener);
    let routes = configure_routes_with_state(route_state(format!("http://127.0.0.1:{port}/v1")));

    let response = routes.handle(&GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"openai/gpt-4o-mini","input":"provider is down"}"#.to_vec(),
    ));
    let json = response_json(&response);
    server.join().unwrap();

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

fn spawn_openai_mock(listener: TcpListener) -> JoinHandle<String> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let request = read_http_request(&mut stream);
        let body = r#"{"id":"chatcmpl-mock","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"mock ok"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .unwrap();
        request
    })
}

fn spawn_closing_server(listener: TcpListener) -> JoinHandle<()> {
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
    })
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "mock provider received EOF before request headers");
        request.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&request) {
            break end;
        }
    };
    let head = std::str::from_utf8(&request[..header_end]).unwrap();
    let content_length = content_length(head);
    while request.len() < header_end + content_length {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "mock provider received EOF before request body");
        request.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(request).unwrap()
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("Content-Length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0)
}

fn provider_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap()
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn bind_loopback_listener() -> Option<TcpListener> {
    for _ in 0..3 {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => return Some(listener),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to bind loopback listener: {error}"),
        }
    }
    None
}
