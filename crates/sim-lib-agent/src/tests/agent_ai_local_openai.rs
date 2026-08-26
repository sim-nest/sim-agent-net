use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use crate::Component;
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, ServerFrame, eval_reply_from_frame};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

#[test]
fn native_local_openai_runners_reflect_provider_defaults() {
    let mut cx = setup_cx();

    let lm_studio = call_runner(&mut cx, "lm-studio", Vec::new());
    let lm_reflected = as_component(&lm_studio).reflect(&mut cx).unwrap();
    assert_eq!(
        map_value(&lm_reflected, "backend"),
        Some(&Expr::Symbol(Symbol::new("lm-studio")))
    );
    assert_eq!(
        map_value(&lm_reflected, "codec"),
        Some(&Expr::Symbol(Symbol::qualified("codec", "lm-studio")))
    );
    assert_eq!(
        map_value(&lm_reflected, "endpoint"),
        Some(&Expr::String("http://127.0.0.1:1234/v1".to_owned()))
    );
    assert_eq!(
        map_value(&lm_reflected, "locality"),
        Some(&Expr::Symbol(Symbol::new("local")))
    );
    assert_eq!(
        map_value(&lm_reflected, "model"),
        Some(&Expr::String("local/default".to_owned()))
    );
    assert!(!capabilities(&lm_reflected).contains(&"ai-runner-secret".to_owned()));

    cx.grant_named("ai-runner-secret");
    let lm_studio_with_auth = call_runner(
        &mut cx,
        "lm-studio",
        vec![(
            ":api-key-env",
            Expr::String("CARGO_MANIFEST_DIR".to_owned()),
        )],
    );
    let lm_auth_reflected = as_component(&lm_studio_with_auth).reflect(&mut cx).unwrap();
    assert_eq!(
        map_value(&lm_auth_reflected, "api-key-env"),
        Some(&Expr::String("CARGO_MANIFEST_DIR".to_owned()))
    );
    assert!(capabilities(&lm_auth_reflected).contains(&"ai-runner-secret".to_owned()));

    let lemonade = call_runner(&mut cx, "lemonade", Vec::new());
    let lemonade_reflected = as_component(&lemonade).reflect(&mut cx).unwrap();
    assert_eq!(
        map_value(&lemonade_reflected, "backend"),
        Some(&Expr::Symbol(Symbol::new("lemonade")))
    );
    assert_eq!(
        map_value(&lemonade_reflected, "codec"),
        Some(&Expr::Symbol(Symbol::qualified("codec", "lemonade")))
    );
    assert_eq!(
        map_value(&lemonade_reflected, "endpoint"),
        Some(&Expr::String("http://127.0.0.1:13305/v1".to_owned()))
    );
    assert_eq!(
        map_value(&lemonade_reflected, "locality"),
        Some(&Expr::Symbol(Symbol::new("local")))
    );
}

#[test]
fn lm_studio_local_only_accepts_loopback_and_rejects_non_loopback() {
    let mut cx = setup_cx();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");

    let Some((port, server)) = spawn_server(|head, body, stream| {
        assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(!head.contains("Authorization: Bearer"));
        assert!(body.contains("\"model\":\"local/default\""));
        write_response(
            stream,
            r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"lm studio ok"}}]}"#,
        );
    }) else {
        return;
    };
    let runner = call_runner(
        &mut cx,
        "lm-studio",
        vec![(
            ":endpoint",
            Expr::String(format!("http://127.0.0.1:{port}/v1")),
        )],
    );
    let request = request_frame(
        &mut cx,
        request_expr(
            "local request",
            vec![key_expr("privacy", Expr::Symbol(Symbol::new("local-only")))],
        ),
    );

    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = reply_expr(&mut cx, &reply);

    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("lm studio ok"));
    assert_eq!(
        map_value(&expr, "provider"),
        Some(&Expr::Symbol(Symbol::new("lm-studio")))
    );
    server.join().unwrap();

    cx.grant_named("ai-runner-network");
    let network_runner = call_runner(
        &mut cx,
        "lm-studio",
        vec![(
            ":endpoint",
            Expr::String("http://models.example/v1".to_owned()),
        )],
    );
    let denied = request_frame(
        &mut cx,
        request_expr(
            "must not encode",
            vec![key_expr("privacy", Expr::Symbol(Symbol::new("local-only")))],
        ),
    );

    let error = as_component(&network_runner)
        .answer(&mut cx, denied)
        .unwrap_err();

    assert!(
        matches!(error, Error::Eval(message) if message.contains("privacy local-only rejected"))
    );
}

#[test]
fn lemonade_runner_accepts_api_v1_base_and_keeps_provider_identity() {
    let mut cx = setup_cx();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");

    let Some((port, server)) = spawn_server(|head, body, stream| {
        assert!(head.starts_with("POST /api/v1/chat/completions HTTP/1.1"));
        assert!(body.contains("\"model\":\"Qwen3-Coder\""));
        write_response(
            stream,
            r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"lemonade ok"}}]}"#,
        );
    }) else {
        return;
    };
    let runner = call_runner(
        &mut cx,
        "lemonade",
        vec![(
            ":endpoint",
            Expr::String(format!("http://127.0.0.1:{port}/api/v1")),
        )],
    );
    let request = request_frame(&mut cx, request_expr("run lemonade", Vec::new()));

    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = reply_expr(&mut cx, &reply);

    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("lemonade ok"));
    assert_eq!(
        map_value(&expr, "provider"),
        Some(&Expr::Symbol(Symbol::new("lemonade")))
    );
    server.join().unwrap();
}

fn setup_cx() -> sim_kernel::Cx {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn call_runner(
    cx: &mut sim_kernel::Cx,
    name: &str,
    options: Vec<(&str, Expr)>,
) -> sim_kernel::Value {
    let values = option_values(cx, options);
    cx.call_function(
        &Symbol::qualified("runner", name),
        sim_kernel::Args::new(values),
    )
    .unwrap()
}

fn option_values(cx: &mut sim_kernel::Cx, options: Vec<(&str, Expr)>) -> Vec<sim_kernel::Value> {
    let mut values = Vec::new();
    for (key, expr) in options {
        values.push(cx.factory().symbol(Symbol::new(key)).unwrap());
        values.push(cx.factory().expr(expr).unwrap());
    }
    values
}

fn request_expr(task: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        key_expr("model-request", Expr::Bool(true)),
        key_expr("task", Expr::String(task.to_owned())),
        key_expr("messages", Expr::List(Vec::new())),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn reply_expr(cx: &mut sim_kernel::Cx, frame: &ServerFrame) -> Expr {
    eval_reply_from_frame(cx, frame)
        .unwrap()
        .value
        .object()
        .as_expr(cx)
        .unwrap()
}

fn map_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
            Some(value)
        }
        _ => None,
    })
}

fn capabilities(expr: &Expr) -> Vec<String> {
    match map_value(expr, "capabilities") {
        Some(Expr::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Expr::String(text) => Some(text.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn spawn_server<F>(handler: F) -> Option<(u16, thread::JoinHandle<()>)>
where
    F: FnOnce(String, String, &mut TcpStream) + Send + 'static,
{
    let listener = bind_loopback_listener()?;
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (head, body) = read_request(&mut stream);
        handler(head, body, &mut stream);
    });
    Some((port, server))
}

fn bind_loopback_listener() -> Option<TcpListener> {
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => Some(listener),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::AddrNotAvailable | ErrorKind::PermissionDenied
            ) =>
        {
            None
        }
        Err(error) => panic!("failed to bind loopback listener: {error}"),
    }
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
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
    let head_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap();
    let head = String::from_utf8_lossy(&request[..head_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = request[head_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    (head, String::from_utf8(body).unwrap())
}

fn write_response(stream: &mut TcpStream, body: &str) {
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
    stream.write_all(body.as_bytes()).unwrap();
}
