#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
use std::{net::TcpListener, thread, time::Duration};

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
use super::*;

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn wait_until(timeout_ms: u64, predicate: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for trigger effect");
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn bind_loopback_listener() -> Option<TcpListener> {
    let mut last_error = None;
    for _ in 0..10 {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => return Some(listener),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to bind loopback listener: {error}"),
        }
    }
    eprintln!(
        "skipping real http trigger bind in this environment: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".to_owned())
    );
    None
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn install_recording_decoder(cx: &mut sim_kernel::Cx) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = cx
        .factory()
        .opaque(Arc::new(RecordFn { seen: seen.clone() }))
        .unwrap();
    let decoder = cx.factory().opaque(Arc::new(DecodeRecordFn)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "record"), record)
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "decode-record"), decoder)
        .unwrap();
    seen
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn start_server(cx: &mut sim_kernel::Cx, name: &str) {
    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", name), server)
        .unwrap();
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn register_trigger(cx: &mut sim_kernel::Cx, server_name: &str, source: Expr) -> sim_kernel::Value {
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "trigger"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::qualified("test", server_name)),
            Expr::Symbol(Symbol::new(":source")),
            quoted(source),
            Expr::Symbol(Symbol::new(":decode")),
            Expr::Symbol(Symbol::qualified("test", "decode-record")),
        ],
    )
    .unwrap()
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn stop_server(cx: &mut sim_kernel::Cx, name: &str) {
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "stop"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", name))],
    )
    .unwrap();
}

#[cfg(all(feature = "server-net-http", feature = "trigger-telegram"))]
#[test]
fn telegram_trigger_polls_real_http_endpoint() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let base_url = format!(
        "http://127.0.0.1:{}/telegram",
        listener.local_addr().unwrap().port()
    );
    let server_thread = spawn_json_http_server(listener, move |count, request| {
        assert_eq!(request.method, "GET");
        assert!(
            request
                .path
                .starts_with("/telegram/botbot-token/getUpdates?")
        );
        if count == 0 {
            json_response(
                br#"{"ok":true,"result":[{"update_id":41,"message":{"chat":{"id":"other"},"text":"skip"}},{"update_id":42,"message":{"chat":{"id":"chat-7"},"text":"hello telegram"}}]}"#
                    .to_vec(),
            )
        } else {
            json_response(br#"{"ok":true,"result":[]}"#.to_vec())
        }
    });

    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("telegram-bot");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "telegram-real-server");

    register_trigger(
        &mut cx,
        "telegram-real-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("telegram")),
            Expr::Symbol(Symbol::new(":chat-id")),
            Expr::String("chat-7".to_owned()),
            Expr::Symbol(Symbol::new(":bot")),
            Expr::String("bot-token".to_owned()),
            Expr::Symbol(Symbol::new(":base-url")),
            Expr::String(base_url),
        ]),
    );

    wait_until(1_000, || {
        seen.lock()
            .expect("telegram seen mutex poisoned")
            .first()
            .map(|text| text.contains("\"update_id\":42"))
            .unwrap_or(false)
    });

    stop_server(&mut cx, "telegram-real-server");
    server_thread.join().unwrap();
}

#[cfg(all(feature = "server-net-http", feature = "trigger-matrix"))]
#[test]
fn matrix_trigger_polls_real_http_sync_endpoint() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let base_url = format!(
        "http://127.0.0.1:{}/matrix",
        listener.local_addr().unwrap().port()
    );
    let server_thread = spawn_json_http_server(listener, move |count, request| {
        assert_eq!(request.method, "GET");
        assert!(request.path.starts_with("/matrix/sync?"));
        if count == 0 {
            json_response(
                br#"{"next_batch":"batch-1","rooms":{"join":{"!room:example":{"timeline":{"events":[{"type":"m.room.message","content":{"body":"matrix hello"}}]}}}}}"#
                    .to_vec(),
            )
        } else {
            json_response(
                br#"{"next_batch":"batch-2","rooms":{"join":{"!room:example":{"timeline":{"events":[]}}}}}"#
                    .to_vec(),
            )
        }
    });

    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("matrix-bot");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "matrix-real-server");

    register_trigger(
        &mut cx,
        "matrix-real-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("matrix")),
            Expr::Symbol(Symbol::new(":room-id")),
            Expr::String("!room:example".to_owned()),
            Expr::Symbol(Symbol::new(":base-url")),
            Expr::String(base_url),
        ]),
    );

    wait_until(1_000, || {
        seen.lock()
            .expect("matrix seen mutex poisoned")
            .first()
            .map(|text| text.contains("matrix hello"))
            .unwrap_or(false)
    });

    stop_server(&mut cx, "matrix-real-server");
    server_thread.join().unwrap();
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn spawn_json_http_server(
    listener: TcpListener,
    handler: impl Fn(usize, crate::http::HttpRequest) -> crate::http::HttpResponse + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut handled = 0usize;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(request) = crate::http::read_request(&mut stream).unwrap() {
                        let response = handler(handled, request);
                        crate::http::write_response(&mut stream, &response).unwrap();
                        handled += 1;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("http test server accept failed: {error}"),
            }
        }
    })
}

#[cfg(all(
    feature = "server-net-http",
    any(feature = "trigger-telegram", feature = "trigger-matrix")
))]
fn json_response(body: Vec<u8>) -> crate::http::HttpResponse {
    crate::http::HttpResponse {
        status: 200,
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body,
    }
}
