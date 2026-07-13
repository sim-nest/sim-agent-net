use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn native_openai_runner_posts_json_decodes_usage_and_keeps_tools_enabled() {
    let mut cx = setup_cx();
    let Some((port, server)) = spawn_openai_server(|head, body, stream| {
        assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(head.contains("Authorization: Bearer "));
        assert!(body.contains("\"model\":\"gpt-5-mini\""));
        assert!(body.contains("\"tools\":[]"));
        assert!(body.contains("Summarize src/lib.rs"));
        write_response(
            stream,
            r#"{"id":"chatcmpl-native","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"native ok"}}],"usage":{"prompt_tokens":9,"completion_tokens":2,"total_tokens":11}}"#,
        );
    }) else {
        return;
    };

    let runner = openai_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let request = request_frame(&mut cx, request_expr("Summarize src/lib.rs"));
    let denied = as_component(&runner)
        .answer(&mut cx, request.clone())
        .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner")
    ));

    grant_openai_caps(&mut cx);
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = reply_expr(&mut cx, &reply);
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("native ok"));
    let usage = field(&expr, "usage").unwrap();
    assert_eq!(number_field(usage, "input-tokens"), Some("9"));
    assert_eq!(number_field(usage, "output-tokens"), Some("2"));
    assert_eq!(number_field(usage, "total-tokens"), Some("11"));
    server.join().unwrap();
}

#[test]
fn native_openai_runner_streams_sse_events() {
    let mut cx = setup_cx();
    grant_openai_caps(&mut cx);
    let Some((port, server)) = spawn_openai_server(|_head, body, stream| {
        assert!(body.contains("\"stream\":true"));
        assert!(body.contains("\"include_usage\":true"));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        for chunk in [
            r#"data: {"choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}"#,
            "data: [DONE]",
        ] {
            let payload = format!("{chunk}\n\n");
            write!(stream, "{:x}\r\n{}\r\n", payload.len(), payload).unwrap();
            stream.flush().unwrap();
        }
        stream.write_all(b"0\r\n\r\n").unwrap();
    }) else {
        return;
    };

    let runner = openai_runner(
        &mut cx,
        port,
        &[
            (":api-key-env", Expr::String("HOME".to_owned())),
            (":stream", Expr::Bool(true)),
        ],
    );
    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("stream"));
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();
    assert_eq!(sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(sink.seen.last(), Some(&FrameKind::StreamEnd));
    assert!(!sink.seen.contains(&FrameKind::Response));
    assert_eq!(
        event_kinds(&sink.chunks),
        vec![
            Expr::Symbol(Symbol::new("start")),
            Expr::Symbol(Symbol::new("delta")),
            Expr::Symbol(Symbol::new("delta")),
            Expr::Symbol(Symbol::new("usage")),
            Expr::Symbol(Symbol::new("final")),
        ]
    );
    assert!(format!("{:?}", sink.chunks).contains("hello"));
    server.join().unwrap();
}

#[test]
fn native_openai_runner_handles_errors_missing_secret_and_redacts_secret() {
    let mut cx = setup_cx();
    grant_openai_caps(&mut cx);

    let missing_env = unique_missing_env();
    let missing = openai_runner(
        &mut cx,
        9,
        &[(":api-key-env", Expr::String(missing_env.clone()))],
    );
    let missing_request = request_frame(&mut cx, request_expr("missing secret"));
    let missing_reply = as_component(&missing)
        .answer(&mut cx, missing_request)
        .unwrap();
    let missing_text = flatten_text(&reply_expr(&mut cx, &missing_reply));
    assert!(
        missing_text.contains("missing env var")
            && missing_text.contains(&missing_env.to_ascii_lowercase()),
        "{missing_text}"
    );

    let Some((port, server)) = spawn_openai_server(|_head, _body, stream| {
        write_response(stream, r#"{"error":{"message":"native error envelope"}}"#);
    }) else {
        return;
    };
    let error_runner = openai_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let error_request = request_frame(&mut cx, request_expr("error envelope"));
    let reply = as_component(&error_runner)
        .answer(&mut cx, error_request)
        .unwrap();
    let expr = reply_expr(&mut cx, &reply);
    assert!(flatten_text(&expr).contains("native error envelope"));
    server.join().unwrap();

    let Some((port, server)) = spawn_openai_server(|_head, _body, stream| {
        let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
        let body = format!("bad bearer {secret}");
        write_http(
            stream,
            "500 Internal Server Error",
            "text/plain",
            body.as_bytes(),
        );
    }) else {
        return;
    };
    let redacted = openai_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let redact_request = request_frame(&mut cx, request_expr("redact"));
    let reply = as_component(&redacted)
        .answer(&mut cx, redact_request)
        .unwrap();
    let expr = reply_expr(&mut cx, &reply);
    let text = flatten_text(&expr);
    let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
    assert!(text.contains("http 500"));
    assert!(!text.contains(&secret));
    server.join().unwrap();
}

fn setup_cx() -> sim_kernel::Cx {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn grant_openai_caps(cx: &mut sim_kernel::Cx) {
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");
}

fn openai_runner(cx: &mut sim_kernel::Cx, port: u16, extra: &[(&str, Expr)]) -> sim_kernel::Value {
    let mut args = vec![
        Expr::Symbol(Symbol::new(":endpoint")),
        Expr::String(format!("http://127.0.0.1:{port}/v1")),
    ];
    for (key, value) in extra {
        args.push(Expr::Symbol(Symbol::new(*key)));
        args.push(value.clone());
    }
    let values = args
        .into_iter()
        .map(|expr| crate::value_from_expr(cx, &expr).unwrap())
        .collect();
    cx.call_function(
        &Symbol::qualified("runner", "openai"),
        sim_kernel::Args::new(values),
    )
    .unwrap()
}

fn reply_expr(cx: &mut sim_kernel::Cx, frame: &ServerFrame) -> Expr {
    eval_reply_from_frame(cx, frame)
        .unwrap()
        .value
        .object()
        .as_expr(cx)
        .unwrap()
}

fn request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
}

fn spawn_openai_server<F>(handler: F) -> Option<(u16, thread::JoinHandle<()>)>
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
    write_http(stream, "200 OK", "application/json", body.as_bytes());
}

fn write_http(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
    stream.write_all(body).unwrap();
}

fn event_kinds(chunks: &[Expr]) -> Vec<Expr> {
    chunks
        .iter()
        .map(|expr| field(expr, "event").unwrap().clone())
        .collect()
}

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
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

fn number_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a str> {
    match field(expr, name)? {
        Expr::Number(number) => Some(number.canonical.as_str()),
        _ => None,
    }
}

fn unique_missing_env() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("SIM_AGENT_NET_OPENAI_MISSING_SECRET_{nonce}")
}

#[derive(Default)]
struct CollectSink {
    chunks: Vec<Expr>,
    seen: Vec<FrameKind>,
}

impl StreamSink for CollectSink {
    fn chunk(&mut self, cx: &mut sim_kernel::Cx, frame: ServerFrame) -> sim_kernel::Result<()> {
        self.seen.push(frame.kind.clone());
        let expr = match frame.kind {
            FrameKind::Response => eval_reply_from_frame(cx, &frame)?
                .value
                .object()
                .as_expr(cx)?,
            FrameKind::StreamChunk => frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?,
            FrameKind::StreamStart | FrameKind::StreamEnd => return Ok(()),
            _ => return Ok(()),
        };
        self.chunks.push(expr);
        Ok(())
    }

    fn end(&mut self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<()> {
        Ok(())
    }
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
