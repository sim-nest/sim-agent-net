use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};
use sim_value::access::field;
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn a5_phase6_openai_compatible_runner_posts_json_and_decodes_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
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
        let head_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .unwrap();
        let head = String::from_utf8_lossy(&request[..head_end]);
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
        let text = String::from_utf8(body).unwrap();
        assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(head.contains(&format!("Authorization: Bearer {secret}")));
        assert!(text.contains("\"model\":\"gpt-4.1-mini\""));
        assert!(text.contains("\"Summarize src/lib.rs\""));
        let body = r#"{"id":"chatcmpl-1","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"remote ok"}}],"usage":{"prompt_tokens":9,"completion_tokens":2}}"#;
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

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("remote")).unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory().string("gpt-4.1-mini".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/v1"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":codec")).unwrap(),
                cx.factory()
                    .symbol(Symbol::qualified("codec", "openai"))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(&mut cx, request_expr("Summarize src/lib.rs"));
    let denied = as_component(&runner)
        .answer(&mut cx, request.clone())
        .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner")
    ));

    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("remote ok"));
    server.join().unwrap();
}

#[test]
fn a5_phase6_openai_compatible_redacts_secret_and_gates_raw_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
    let port = listener.local_addr().unwrap().port();
    let expected_redaction = secret.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request).unwrap();
        let body = format!("bad bearer {secret}");
        stream
            .write_all(
                format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .unwrap();
    });

    let failing = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/v1"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let fail_request = request_frame(&mut cx, request_expr("leak check"));
    let fail_reply = as_component(&failing)
        .answer(&mut cx, fail_request)
        .unwrap();
    let fail_expr = eval_reply_from_frame(&mut cx, &fail_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let fail_text = flatten_text(&fail_expr);
    assert!(fail_text.contains("http 500"));
    assert!(!fail_text.contains(&expected_redaction));
    server.join().unwrap();

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let body = r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"raw ok"}}],"usage":{"prompt_tokens":1}}"#;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request).unwrap();
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
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/v1"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let raw_request = request_frame(&mut cx, request_expr("raw"));
    let reply = as_component(&runner).answer(&mut cx, raw_request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(!format!("{expr:?}").contains("raw-provider-response"));
    server.join().unwrap();

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let body = r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"raw ok"}}],"usage":{"prompt_tokens":1}}"#;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request).unwrap();
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
    cx.grant_named("ai-runner-raw-log");
    let raw_runner = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/v1"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let raw_request = request_frame(&mut cx, request_expr("raw"));
    let raw_reply = as_component(&raw_runner)
        .answer(&mut cx, raw_request)
        .unwrap();
    let raw_expr = eval_reply_from_frame(&mut cx, &raw_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(format!("{raw_expr:?}").contains("raw-provider-response"));
    server.join().unwrap();
}

#[test]
fn a6_phase3_openai_compatible_streams_sse_events() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");

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
        let head_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .unwrap();
        let head = String::from_utf8_lossy(&request[..head_end]);
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
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("\"stream\":true"));
        assert!(text.contains("\"include_usage\":true"));
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
    });

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/v1"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":stream")).unwrap(),
                cx.factory().bool(true).unwrap(),
            ]),
        )
        .unwrap();
    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("stream"));
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();
    assert_eq!(sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(sink.seen.last(), Some(&FrameKind::StreamEnd));
    assert!(!sink.seen.contains(&FrameKind::Response));
    assert_eq!(
        sink.seen
            .iter()
            .filter(|kind| **kind == FrameKind::StreamChunk)
            .count(),
        sink.chunks.len()
    );
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
    let joined = format!("{:?}", sink.chunks);
    assert!(joined.contains("hello"));
    server.join().unwrap();
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

fn event_kinds(chunks: &[Expr]) -> Vec<Expr> {
    chunks
        .iter()
        .map(|expr| field(expr, "event").unwrap().clone())
        .collect()
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
