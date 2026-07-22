use super::{HttpRunner, anthropic_headers};
use crate::{ProviderAuth, ProviderConfig, provider_profiles};
use sim_kernel::{CapabilityName, CapabilitySet, Cx, DefaultFactory, EagerPolicy, Expr, Symbol};
use sim_lib_agent_runner_core::{
    ModelRequest, ModelRunner, OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA,
    OUTPUT_GRAMMAR_REQUIRED_EXTRA, RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA,
};
use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

#[test]
fn new_provider_maps_config_onto_existing_runner_fields() {
    let profile = provider_profiles::anthropic();
    let config = ProviderConfig {
        profile: profile.clone(),
        runner: Symbol::new("claude"),
        codec: Symbol::qualified("codec", "anthropic"),
        endpoint: "https://api.anthropic.com/v1".to_owned(),
        model: "claude-sonnet-latest".to_owned(),
        api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
        locality: Symbol::new("network"),
        timeout: Duration::from_secs(45),
        stream: true,
        tools: true,
        max_output_bytes: 8192,
        grammar_dialects: profile.grammar_dialects.clone(),
    };

    let runner = HttpRunner::new_provider(config);

    assert_eq!(runner.runner, Symbol::new("claude"));
    assert_eq!(runner.model, "claude-sonnet-latest");
    assert_eq!(runner.provider, Symbol::new("anthropic"));
    assert_eq!(runner.locality, Symbol::new("network"));
    assert_eq!(runner.runner_label, "runner/provider");
    assert_eq!(runner.request_path, "/messages");
    assert_eq!(runner.endpoint, "https://api.anthropic.com/v1");
    assert_eq!(runner.api_key_env, Some("ANTHROPIC_API_KEY".to_owned()));
    assert_eq!(
        runner.auth,
        ProviderAuth::HeaderEnv {
            header: "x-api-key".to_owned(),
            env: "ANTHROPIC_API_KEY".to_owned()
        }
    );
    assert_eq!(runner.codec, Symbol::qualified("codec", "anthropic"));
    assert_eq!(runner.timeout, Duration::from_secs(45));
    assert!(runner.stream);
    assert!(runner.tools);
    assert_eq!(runner.max_response_bytes, 8192);
    assert!(runner.grammar_dialects.is_empty());
    assert_eq!(profile.chat_path, "/messages");
}

#[test]
fn new_provider_card_uses_provider_and_locality() {
    let mut cx = test_cx();
    let config =
        ProviderConfig::from_options(provider_profiles::ollama(), &mut cx, &HashMap::new())
            .unwrap();
    let card = HttpRunner::new_provider(config).card();

    assert_eq!(card.runner, Symbol::qualified("runner", "ollama"));
    assert_eq!(card.provider, Symbol::new("ollama"));
    assert_eq!(card.locality, Symbol::new("local"));
    assert!(format!("{:?}", card.extra).contains("gbnf"));
}

#[test]
fn openai_provider_selects_json_schema_output_dialect() {
    let profile = provider_profiles::openai();
    let runner = HttpRunner::new_provider(ProviderConfig {
        profile: profile.clone(),
        runner: profile.runner_symbol.clone(),
        codec: profile.codec.clone(),
        endpoint: "http://127.0.0.1:9/v1".to_owned(),
        model: "gpt-test".to_owned(),
        api_key_env: Some("CARGO_MANIFEST_DIR".to_owned()),
        locality: Symbol::new("network"),
        timeout: Duration::from_secs(1),
        stream: false,
        tools: false,
        max_output_bytes: 64 * 1024,
        grammar_dialects: profile.grammar_dialects,
    });

    let request = runner.prepare_output_grammar(shape_model_request());

    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_DIALECT_EXTRA),
        Some(&Expr::Symbol(Symbol::new("json-schema")))
    );
    assert!(extra(&request, OUTPUT_GRAMMAR_EXTRA).is_none());
}

#[test]
fn ollama_provider_selects_gbnf_output_dialect() {
    let runner = HttpRunner::new_ollama(
        Symbol::qualified("runner", "ollama"),
        "qwen-test",
        Symbol::new("local"),
        "http://127.0.0.1:11434",
        Symbol::qualified("codec", "ollama"),
        Duration::from_secs(1),
        false,
        false,
        64 * 1024,
    );

    let request = runner.prepare_output_grammar(shape_model_request());

    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_DIALECT_EXTRA),
        Some(&Expr::Symbol(Symbol::new("gbnf")))
    );
    assert!(extra(&request, OUTPUT_GRAMMAR_EXTRA).is_none());
}

#[test]
fn provider_without_grammar_support_strips_grammar_for_repair() {
    let profile = provider_profiles::anthropic();
    let runner = HttpRunner::new_provider(ProviderConfig {
        profile: profile.clone(),
        runner: profile.runner_symbol.clone(),
        codec: profile.codec.clone(),
        endpoint: "http://127.0.0.1:9/v1".to_owned(),
        model: "claude-test".to_owned(),
        api_key_env: Some("CARGO_MANIFEST_DIR".to_owned()),
        locality: Symbol::new("network"),
        timeout: Duration::from_secs(1),
        stream: false,
        tools: false,
        max_output_bytes: 64 * 1024,
        grammar_dialects: profile.grammar_dialects,
    });

    let request = runner.prepare_output_grammar(shape_model_request_with_stale_grammar());

    assert!(extra(&request, OUTPUT_GRAMMAR_EXTRA).is_none());
    assert!(extra(&request, OUTPUT_GRAMMAR_DIALECT_EXTRA).is_none());
}

#[test]
fn anthropic_headers_include_secret_version_and_json_content_type() {
    assert_eq!(
        anthropic_headers("secret-token"),
        vec![
            ("x-api-key".to_owned(), "secret-token".to_owned()),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]
    );
}

#[test]
fn direct_http_runner_denies_without_runner_capabilities() {
    let runner = HttpRunner::new_openai_compatible(
        Symbol::qualified("runner", "direct-denied"),
        "gpt-test",
        "http://127.0.0.1:1/v1",
        "CARGO_MANIFEST_DIR",
        Symbol::qualified("codec", "openai"),
        Duration::from_secs(1),
        false,
        false,
        64 * 1024,
    );
    let mut cx = test_cx();

    let response = runner
        .infer(
            &mut cx,
            ModelRequest::new(Expr::String("denied".to_owned()), Vec::new()),
        )
        .unwrap();

    assert_eq!(response.stop_reason, Symbol::new("error"));
    assert!(format!("{:?}", response.content).contains("ai-runner"));
}

#[test]
fn direct_http_runner_allows_with_runner_network_and_secret_capabilities() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let server = spawn_openai_mock(listener);
    let runner = HttpRunner::new_openai_compatible(
        Symbol::qualified("runner", "direct-allowed"),
        "gpt-test",
        format!("http://127.0.0.1:{port}/v1"),
        "CARGO_MANIFEST_DIR",
        Symbol::qualified("codec", "openai"),
        Duration::from_secs(2),
        false,
        false,
        64 * 1024,
    );
    let mut cx = test_cx();
    let capabilities = CapabilitySet::new()
        .grant(CapabilityName::new("ai-runner"))
        .grant(CapabilityName::new("ai-runner-network"))
        .grant(CapabilityName::new("ai-runner-secret"));

    let response = cx
        .with_capabilities(capabilities, |cx| {
            runner.infer(
                cx,
                ModelRequest::new(Expr::String("allowed direct".to_owned()), Vec::new()),
            )
        })
        .unwrap();
    let request = server.join().unwrap();

    assert_eq!(response.stop_reason, Symbol::new("stop"));
    assert!(format!("{:?}", response.content).contains("direct ok"));
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request.contains("allowed direct"));
}

fn test_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

fn shape_model_request() -> ModelRequest {
    let mut request = ModelRequest::new(Expr::String("shape please".to_owned()), Vec::new());
    request.extra.push(entry(
        RETURN_CODEC_EXTRA,
        Expr::Symbol(Symbol::qualified("codec", "json")),
    ));
    request.extra.push(entry(
        RETURN_SHAPE_EXTRA,
        Expr::Symbol(Symbol::new("String")),
    ));
    request
        .extra
        .push(entry(OUTPUT_GRAMMAR_REQUIRED_EXTRA, Expr::Bool(true)));
    request
}

fn shape_model_request_with_stale_grammar() -> ModelRequest {
    let mut request = shape_model_request();
    request.extra.push(entry(
        OUTPUT_GRAMMAR_EXTRA,
        Expr::String(r#"{"type":"stale"}"#.to_owned()),
    ));
    request.extra.push(entry(
        OUTPUT_GRAMMAR_DIALECT_EXTRA,
        Expr::Symbol(Symbol::new("json-schema")),
    ));
    request
}

fn entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn extra<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a Expr> {
    request.extra.iter().find_map(|(key, value)| {
        matches!(key, Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name)
            .then_some(value)
    })
}

fn spawn_openai_mock(listener: TcpListener) -> JoinHandle<String> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let request = read_http_request(&mut stream);
        let body = r#"{"id":"chatcmpl-direct","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"direct ok"}}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#;
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
