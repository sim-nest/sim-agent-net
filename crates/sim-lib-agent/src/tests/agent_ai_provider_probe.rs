use super::support::{eval_cx, install_agent_lib};
use sim_kernel::{Args, Error, Expr, Result, Symbol, Value};
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn provider_profiles_returns_profile_table() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();

    let value = cx
        .call_function(
            &Symbol::qualified("provider", "profiles"),
            Args::new(Vec::new()),
        )
        .unwrap();
    let expr = value.object().as_expr(&mut cx).unwrap();

    let openai = map_field(&expr, "openai");
    assert_eq!(
        map_field(openai, "models-path"),
        &Expr::String("/models".to_owned())
    );
    assert_eq!(
        map_field(openai, "runner"),
        &Expr::Symbol(Symbol::qualified("runner", "openai"))
    );
    assert_eq!(
        map_field(map_field(openai, "auth"), "kind"),
        &Expr::Symbol(Symbol::new("bearer-env"))
    );
    assert!(map_field(&expr, "ollama").canonical_eq(&Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("provider")),
            Expr::Symbol(Symbol::new("ollama"))
        ),
        (
            Expr::Symbol(Symbol::new("runner")),
            Expr::Symbol(Symbol::qualified("runner", "ollama"))
        ),
        (
            Expr::Symbol(Symbol::new("codec")),
            Expr::Symbol(Symbol::qualified("codec", "ollama"))
        ),
        (
            Expr::Symbol(Symbol::new("endpoint")),
            Expr::String("http://127.0.0.1:11434".to_owned())
        ),
        (
            Expr::Symbol(Symbol::new("models-path")),
            Expr::String("/api/tags".to_owned())
        ),
        (
            Expr::Symbol(Symbol::new("chat-path")),
            Expr::String("/api/chat".to_owned())
        ),
        (
            Expr::Symbol(Symbol::new("auth")),
            Expr::Map(vec![(
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(Symbol::new("none"))
            )])
        ),
        (
            Expr::Symbol(Symbol::new("locality")),
            Expr::Symbol(Symbol::new("local"))
        ),
        (
            Expr::Symbol(Symbol::new("model")),
            Expr::String("qwen3.5:4b".to_owned())
        ),
        (
            Expr::Symbol(Symbol::new("timeout-ms")),
            number_expr("120000")
        ),
        (Expr::Symbol(Symbol::new("stream")), Expr::Bool(true)),
        (Expr::Symbol(Symbol::new("tools")), Expr::Bool(false)),
        (
            Expr::Symbol(Symbol::new("max-output-bytes")),
            number_expr("1048576")
        ),
    ])));
}

#[test]
fn provider_probe_loopback_requires_local_capability() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");

    let error = call_probe(
        &mut cx,
        vec![
            ("provider", Expr::Symbol(Symbol::new("lm-studio"))),
            (
                "endpoint",
                Expr::String("http://127.0.0.1:1234/v1".to_owned()),
            ),
        ],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner-local")
    ));
}

#[test]
fn provider_probe_non_loopback_requires_network_capability() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");

    let error = call_probe(
        &mut cx,
        vec![
            ("provider", Expr::Symbol(Symbol::new("openai-compatible"))),
            (
                "endpoint",
                Expr::String("http://models.example/v1".to_owned()),
            ),
            ("api-key-env", Expr::Nil),
        ],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner-network")
    ));
}

#[test]
fn provider_probe_secret_env_is_required_before_missing_secret_is_reported() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");

    let denied = call_probe(
        &mut cx,
        vec![
            ("provider", Expr::Symbol(Symbol::new("openai"))),
            (
                "api-key-env",
                Expr::String("SIM_AGENT_NET_PROVIDER_PROBE_MISSING_SECRET".to_owned()),
            ),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner-secret")
    ));

    cx.grant_named("ai-runner-secret");
    let report = call_probe(
        &mut cx,
        vec![
            ("provider", Expr::Symbol(Symbol::new("openai"))),
            (
                "api-key-env",
                Expr::String("SIM_AGENT_NET_PROVIDER_PROBE_MISSING_SECRET".to_owned()),
            ),
        ],
    )
    .unwrap();

    assert_eq!(
        map_field(&report, "status"),
        &Expr::Symbol(Symbol::new("skipped"))
    );
    assert_eq!(map_field(&report, "redacted"), &Expr::Bool(true));
    assert_eq!(
        map_field(&report, "reason"),
        &Expr::String("missing secret env SIM_AGENT_NET_PROVIDER_PROBE_MISSING_SECRET".to_owned())
    );
}

#[test]
fn provider_probe_loopback_discovers_models_without_live_provider() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&request);
        assert!(head.starts_with("GET /v1/models HTTP/1.1"), "{head}");
        let body = r#"{"data":[{"id":"local-a"},{"id":"local-b"}]}"#;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .unwrap();
    });

    let report = call_probe(
        &mut cx,
        vec![
            ("provider", Expr::Symbol(Symbol::new("lm-studio"))),
            (
                "endpoint",
                Expr::String(format!("http://127.0.0.1:{port}/v1")),
            ),
        ],
    )
    .unwrap();

    assert_eq!(
        map_field(&report, "status"),
        &Expr::Symbol(Symbol::new("available"))
    );
    assert_eq!(
        list_strings(map_field(&report, "models")),
        vec!["local-a".to_owned(), "local-b".to_owned()]
    );
    assert_eq!(map_field(&report, "redacted"), &Expr::Bool(false));
    server.join().unwrap();
}

fn call_probe(cx: &mut sim_kernel::Cx, options: Vec<(&str, Expr)>) -> Result<Expr> {
    let args = Args::new(option_values(cx, options));
    let value = cx.call_function(&Symbol::qualified("provider", "probe"), args)?;
    value.object().as_expr(cx)
}

fn option_values(cx: &mut sim_kernel::Cx, options: Vec<(&str, Expr)>) -> Vec<Value> {
    let mut values = Vec::new();
    for (key, expr) in options {
        values.push(cx.factory().symbol(Symbol::new(format!(":{key}"))).unwrap());
        values.push(cx.factory().expr(expr).unwrap());
    }
    values
}

fn map_field<'a>(expr: &'a Expr, key: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map expr, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(field, value)| match field {
            Expr::Symbol(symbol) if symbol.to_string() == key => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing map field {key} in {expr:?}"))
}

fn list_strings(expr: &Expr) -> Vec<String> {
    let Expr::List(items) = expr else {
        panic!("expected list expr, found {expr:?}");
    };
    items
        .iter()
        .map(|item| match item {
            Expr::String(text) => text.clone(),
            other => panic!("expected string model, found {other:?}"),
        })
        .collect()
}

fn number_expr(canonical: &str) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: canonical.to_owned(),
    })
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
