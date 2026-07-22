use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};
use sim_value::access::field;
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn native_anthropic_runner_posts_messages_with_required_headers() {
    let mut cx = setup_cx();
    grant_anthropic_caps(&mut cx);
    let Some((port, server)) = spawn_anthropic_server(|head, body, stream| {
        assert!(head.starts_with("POST /v1/messages HTTP/1.1"));
        assert!(head.contains("x-api-key: "));
        assert!(head.contains("anthropic-version: 2023-06-01"));
        assert!(head.contains("content-type: application/json"));
        assert!(!head.contains("Authorization: Bearer"));
        assert!(body.contains("\"model\":\"claude-sonnet-latest\""));
        assert!(body.contains("\"max_tokens\":1024"));
        assert!(body.contains("Summarize src/lib.rs"));
        write_response(
            stream,
            r#"{"id":"msg_native","type":"message","role":"assistant","model":"claude-sonnet-latest","content":[{"type":"text","text":"anthropic ok"}],"stop_reason":"end_turn","usage":{"input_tokens":7,"output_tokens":2}}"#,
        );
    }) else {
        return;
    };

    let runner = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let request = request_frame(&mut cx, request_expr("Summarize src/lib.rs", Vec::new()));
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = reply_expr(&mut cx, &reply);

    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("anthropic ok"));
    let usage = field(&expr, "usage").unwrap();
    assert_eq!(number_value(usage, "input-tokens"), Some("7"));
    assert_eq!(number_value(usage, "output-tokens"), Some("2"));
    server.join().unwrap();
}

#[test]
fn native_anthropic_runner_streams_sse_deltas() {
    let mut cx = setup_cx();
    grant_anthropic_caps(&mut cx);
    let Some((port, server)) = spawn_anthropic_server(|_head, body, stream| {
        assert!(body.contains("\"stream\":true"));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        for chunk in [
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-latest\",\"content\":[]}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hel\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ] {
            write_chunk(stream, chunk);
        }
        stream.write_all(b"0\r\n\r\n").unwrap();
    }) else {
        return;
    };

    let runner = anthropic_runner(
        &mut cx,
        port,
        &[
            (":api-key-env", Expr::String("HOME".to_owned())),
            (":stream", Expr::Bool(true)),
        ],
    );
    let request = request_frame(&mut cx, request_expr("stream", Vec::new()));
    let mut sink = CollectSink::default();
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();

    assert_eq!(sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(sink.seen.last(), Some(&FrameKind::StreamEnd));
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
fn native_anthropic_runner_decodes_tool_use() {
    let mut cx = setup_cx();
    grant_anthropic_caps(&mut cx);
    let Some((port, server)) = spawn_anthropic_server(|_head, body, stream| {
        assert!(body.contains("\"tools\""));
        assert!(body.contains("get_weather"));
        write_response(
            stream,
            r#"{"id":"msg_tool","type":"message","role":"assistant","model":"claude-sonnet-latest","content":[{"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"location":"Stockholm"}}],"stop_reason":"tool_use","usage":{"input_tokens":9,"output_tokens":4}}"#,
        );
    }) else {
        return;
    };

    let runner = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let request = request_frame(
        &mut cx,
        request_expr(
            "use the weather tool",
            vec![key_expr("tools", Expr::List(vec![weather_tool_schema()]))],
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = reply_expr(&mut cx, &reply);
    let rendered = format!("{expr:?}");

    validate_chat_transcript(&expr).unwrap();
    assert!(rendered.contains("tool-call"));
    assert!(rendered.contains("toolu_1"));
    assert!(rendered.contains("get_weather"));
    assert!(rendered.contains("Stockholm"));
    server.join().unwrap();
}

#[test]
fn native_anthropic_runner_handles_errors_missing_secret_and_redacts_secret() {
    let mut cx = setup_cx();
    grant_anthropic_caps(&mut cx);

    let listener = bind_loopback_listener().unwrap();
    let port = listener.local_addr().unwrap().port();
    let missing_env = unique_missing_env();
    let missing = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String(missing_env.clone()))],
    );
    let missing_request = request_frame(&mut cx, request_expr("missing secret", Vec::new()));
    let missing_reply = as_component(&missing)
        .answer(&mut cx, missing_request)
        .unwrap();
    let missing_text = flatten_text(&reply_expr(&mut cx, &missing_reply));
    assert!(
        missing_text.contains("missing env var")
            && missing_text.contains(&missing_env.to_ascii_lowercase()),
        "{missing_text}"
    );
    assert_no_connection(listener);

    let Some((port, server)) = spawn_anthropic_server(|_head, _body, stream| {
        write_response(
            stream,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"anthropic error envelope"}}"#,
        );
    }) else {
        return;
    };
    let error_runner = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let error_request = request_frame(&mut cx, request_expr("error envelope", Vec::new()));
    let reply = as_component(&error_runner)
        .answer(&mut cx, error_request)
        .unwrap();
    assert!(flatten_text(&reply_expr(&mut cx, &reply)).contains("anthropic error envelope"));
    server.join().unwrap();

    let Some((port, server)) = spawn_anthropic_server(|_head, _body, stream| {
        let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
        let body = format!("bad key {secret}");
        write_http(
            stream,
            "500 Internal Server Error",
            "text/plain",
            body.as_bytes(),
        );
    }) else {
        return;
    };
    let redacted = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let redact_request = request_frame(&mut cx, request_expr("redact", Vec::new()));
    let reply = as_component(&redacted)
        .answer(&mut cx, redact_request)
        .unwrap();
    let text = flatten_text(&reply_expr(&mut cx, &reply));
    let secret = std::env::var("HOME").unwrap_or_else(|_| "/tmp/sim-home".to_owned());
    assert!(text.contains("http 500"));
    assert!(!text.contains(&secret));
    server.join().unwrap();
}

#[test]
fn native_anthropic_runner_rejects_local_only_before_encoding() {
    let mut cx = setup_cx();
    grant_anthropic_caps(&mut cx);
    let listener = bind_loopback_listener().unwrap();
    let port = listener.local_addr().unwrap().port();
    let runner = anthropic_runner(
        &mut cx,
        port,
        &[(":api-key-env", Expr::String("HOME".to_owned()))],
    );
    let frame = request_frame(
        &mut cx,
        Expr::Map(vec![
            key_expr("privacy", Expr::Symbol(Symbol::new("local-only"))),
            key_expr("task", Expr::String("invalid request shape".to_owned())),
        ]),
    );

    let error = as_component(&runner).answer(&mut cx, frame).unwrap_err();

    assert!(
        matches!(error, Error::Eval(message) if message.contains("privacy local-only rejected"))
    );
    assert_no_connection(listener);
}

fn setup_cx() -> sim_kernel::Cx {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn grant_anthropic_caps(cx: &mut sim_kernel::Cx) {
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");
}

fn anthropic_runner(
    cx: &mut sim_kernel::Cx,
    port: u16,
    extra: &[(&str, Expr)],
) -> sim_kernel::Value {
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
        &Symbol::qualified("runner", "anthropic"),
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

fn weather_tool_schema() -> Expr {
    Expr::Map(vec![
        key_expr("name", Expr::String("get_weather".to_owned())),
        key_expr(
            "description",
            Expr::String("Read a city forecast.".to_owned()),
        ),
        key_expr(
            "parameters",
            Expr::Map(vec![
                key_expr("type", Expr::String("object".to_owned())),
                key_expr(
                    "properties",
                    Expr::Map(vec![key_expr(
                        "location",
                        Expr::Map(vec![key_expr("type", Expr::String("string".to_owned()))]),
                    )]),
                ),
            ]),
        ),
    ])
}

fn spawn_anthropic_server<F>(handler: F) -> Option<(u16, thread::JoinHandle<()>)>
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

fn write_chunk(stream: &mut TcpStream, chunk: &str) {
    write!(stream, "{:x}\r\n{}\r\n", chunk.len(), chunk).unwrap();
    stream.flush().unwrap();
}

fn event_kinds(chunks: &[Expr]) -> Vec<Expr> {
    chunks
        .iter()
        .map(|expr| field(expr, "event").unwrap().clone())
        .collect()
}

fn number_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a str> {
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
    format!("SIM_AGENT_NET_ANTHROPIC_MISSING_SECRET_{nonce}")
}

fn assert_no_connection(listener: TcpListener) {
    listener.set_nonblocking(true).unwrap();
    match listener.accept() {
        Ok(_) => panic!("runner opened a socket before the request was allowed"),
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        Err(error) => panic!("unexpected listener error: {error}"),
    }
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
