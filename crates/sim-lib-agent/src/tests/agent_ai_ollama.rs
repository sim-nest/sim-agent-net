use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use crate::Component;
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
fn a6_phase1_ollama_runner_posts_chat_payload_and_decodes_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

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
        assert!(head.starts_with("POST /api/chat HTTP/1.1"));
        assert!(!head.contains("Authorization: Bearer"));
        assert!(text.contains("\"model\":\"qwen3.5:4b\""));
        assert!(text.contains("\"Summarize src/lib.rs\""));
        let body = r#"{"model":"qwen3.5:4b","message":{"role":"assistant","content":"local ok"},"done":true,"done_reason":"stop","prompt_eval_count":7,"eval_count":2}"#;
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
            &Symbol::qualified("runner", "ollama"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("local-qwen")).unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory().string("qwen3.5:4b".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}"))
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
    cx.grant_named("ai-runner-local");
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("local ok"));
    server.join().unwrap();
}

#[test]
fn a6_phase1_ollama_runner_buffers_streaming_ndjson_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-local");

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request).unwrap();
        let body = concat!(
            "{\"model\":\"qwen3.5:4b\",\"message\":{\"role\":\"assistant\",\"content\":\"hello \"},\"done\":false}\n",
            "{\"model\":\"qwen3.5:4b\",\"message\":{\"role\":\"assistant\",\"content\":\"ollama\"},\"done\":false}\n",
            "{\"model\":\"qwen3.5:4b\",\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":6,\"eval_count\":2}\n"
        );
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .unwrap();
    });

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "ollama"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":stream")).unwrap(),
                cx.factory().bool(true).unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(&mut cx, request_expr("stream"));
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("hello ollama"));
    server.join().unwrap();
}

#[test]
fn a6_phase3_ollama_runner_streams_ndjson_events() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
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
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        for chunk in [
            "{\"model\":\"qwen3.5:4b\",\"message\":{\"role\":\"assistant\",\"content\":\"hello \"},\"done\":false}\n",
            "{\"model\":\"qwen3.5:4b\",\"message\":{\"role\":\"assistant\",\"content\":\"ollama\"},\"done\":false}\n",
            "{\"model\":\"qwen3.5:4b\",\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":6,\"eval_count\":2}\n",
        ] {
            write!(stream, "{:x}\r\n{}\r\n", chunk.len(), chunk).unwrap();
            stream.flush().unwrap();
        }
        stream.write_all(b"0\r\n\r\n").unwrap();
    });

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "ollama"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}"))
                    .unwrap(),
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
    assert!(joined.contains("hello ollama"));
    server.join().unwrap();
}

#[test]
fn runner_ollama_uses_native_provider_profile() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "ollama"),
            sim_kernel::Args::new(Vec::new()),
        )
        .unwrap();
    let reflected = as_component(&runner).reflect(&mut cx).unwrap();

    assert_eq!(
        field(&reflected, "backend"),
        Some(&Expr::Symbol(Symbol::new("ollama")))
    );
    assert_eq!(
        field(&reflected, "provider"),
        Some(&Expr::Symbol(Symbol::new("ollama")))
    );
    assert_eq!(
        field(&reflected, "codec"),
        Some(&Expr::Symbol(Symbol::qualified("codec", "ollama")))
    );
    assert_eq!(
        field(&reflected, "endpoint"),
        Some(&Expr::String("http://127.0.0.1:11434".to_owned()))
    );
    assert_eq!(
        field(&reflected, "locality"),
        Some(&Expr::Symbol(Symbol::new("local")))
    );
    assert_eq!(
        field(&reflected, "model"),
        Some(&Expr::String("qwen3.5:4b".to_owned()))
    );
    assert_eq!(field(&reflected, "stream"), Some(&Expr::Bool(true)));
    assert_eq!(field(&reflected, "tools"), Some(&Expr::Bool(false)));
    assert!(capabilities(&reflected).contains(&"ai-runner".to_owned()));
    assert!(capabilities(&reflected).contains(&"ai-runner-local".to_owned()));
    assert!(!capabilities(&reflected).contains(&"ai-runner-secret".to_owned()));
    assert_ne!(
        field(&reflected, "backend"),
        Some(&Expr::Symbol(Symbol::new("openai-compatible")))
    );
    assert_ne!(
        field(&reflected, "codec"),
        Some(&Expr::Symbol(Symbol::qualified("codec", "openai")))
    );
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

fn capabilities(expr: &Expr) -> Vec<String> {
    match field(expr, "capabilities") {
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

fn request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("role")),
                    Expr::Symbol(Symbol::new("system")),
                ),
                (
                    Expr::Symbol(Symbol::new("content")),
                    Expr::List(vec![Expr::Map(vec![
                        (
                            Expr::Symbol(Symbol::new("type")),
                            Expr::Symbol(Symbol::new("text")),
                        ),
                        (
                            Expr::Symbol(Symbol::new("text")),
                            Expr::String("Be concise.".to_owned()),
                        ),
                    ])]),
                ),
            ])]),
        ),
    ])
}
