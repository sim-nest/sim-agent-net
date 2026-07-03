use super::support::{
    as_component, eval_cx, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use sim_kernel::{Expr, Symbol};
use sim_lib_server::{EvalSite, eval_reply_from_frame};
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn a6_phase6_local_only_rejects_http_runner_before_encoding() {
    let mut cx = http_privacy_cx();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "openai-compatible"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string("http://127.0.0.1:9/v1".to_owned())
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":api-key-env")).unwrap(),
                cx.factory().string("HOME".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let frame = request_frame(
        &mut cx,
        request_expr(
            "must not encode",
            vec![key_expr("privacy", Expr::Symbol(Symbol::new("local-only")))],
        ),
    );

    let error = as_component(&runner).answer(&mut cx, frame).unwrap_err();

    assert!(error.to_string().contains("privacy local-only rejected"));
}

#[test]
fn a6_phase6_no_raw_suppresses_http_raw_capture() {
    let mut cx = http_privacy_cx();
    cx.grant_named("ai-runner-raw-log");

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let body = r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"private raw ok"}}],"usage":{"prompt_tokens":1}}"#;
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
    let frame = request_frame(
        &mut cx,
        request_expr(
            "raw denied",
            vec![key_expr("privacy", Expr::Symbol(Symbol::new("no-raw")))],
        ),
    );

    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert!(!format!("{expr:?}").contains("raw-provider-response"));
    server.join().unwrap();
}

fn http_privacy_cx() -> sim_kernel::Cx {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-network");
    cx.grant_named("ai-runner-secret");
    cx
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
