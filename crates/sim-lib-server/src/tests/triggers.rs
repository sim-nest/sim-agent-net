use super::*;
#[cfg(all(feature = "server-net-http", feature = "trigger-webhook"))]
use crate::transport::port_io::PortTcpStream;
#[cfg(all(feature = "server-net-http", feature = "trigger-webhook"))]
use std::io::Write;

fn wait_until(timeout_ms: u64, predicate: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(timeout_ms) {
        if predicate() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for trigger effect");
}

#[test]
fn file_tail_trigger_decodes_lisp_and_reflects_registration() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("file-read");

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

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "trigger-server"), server.clone())
        .unwrap();

    let path = std::env::temp_dir().join(format!(
        "sim-lib-server-trigger-{}.lisp",
        NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, "").unwrap();

    let handle = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "trigger"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "trigger-server")),
                Expr::Symbol(Symbol::new(":source")),
                quoted(Expr::List(vec![
                    Expr::Symbol(Symbol::new("file-tail")),
                    Expr::Symbol(Symbol::new(":path")),
                    Expr::String(path.display().to_string()),
                ])),
                Expr::Symbol(Symbol::new(":decode")),
                Expr::Symbol(Symbol::qualified("test", "decode-record")),
            ],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "file-trigger"), handle)
        .unwrap();

    fs::write(&path, "alpha\n").unwrap();
    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice() == ["alpha".to_owned()]
    });
    assert_eq!(
        seen.lock().expect("record fn mutex poisoned").as_slice(),
        &["alpha".to_owned()]
    );
    let trigger = cx
        .eval_expr(Expr::Symbol(Symbol::qualified("test", "file-trigger")))
        .unwrap();
    let reflected = normalized_reflect_table(&mut cx, trigger);
    assert_eq!(
        reflected.get("delivered"),
        Some(&Expr::String("1".to_owned()))
    );

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "stop"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", "trigger-server"))],
    )
    .unwrap();

    let _ = fs::remove_file(path);
}

#[test]
fn stdin_trigger_injects_text_events_through_callable_decoder() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

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

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "stdin-server"), server)
        .unwrap();

    let handle = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "trigger"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "stdin-server")),
                Expr::Symbol(Symbol::new(":source")),
                quoted(Expr::Symbol(Symbol::new("stdin"))),
                Expr::Symbol(Symbol::new(":decode")),
                Expr::Symbol(Symbol::qualified("test", "decode-record")),
            ],
        )
        .unwrap();
    let trigger = handle.object().downcast_ref::<TriggerHandle>().unwrap();
    trigger.feed_stdin("beta\n").unwrap();
    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice() == ["beta".to_owned()]
    });
    assert_eq!(
        seen.lock().expect("record fn mutex poisoned").as_slice(),
        &["beta".to_owned()]
    );
    trigger.finish_stdin().unwrap();
    wait_until(1_000, || trigger.is_source_closed());
}

#[test]
fn file_tail_trigger_replays_after_truncation() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("file-read");

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

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "truncate-server"), server)
        .unwrap();

    let path = std::env::temp_dir().join(format!(
        "sim-lib-server-trigger-reset-{}.lisp",
        NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, "").unwrap();

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "trigger"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::qualified("test", "truncate-server")),
            Expr::Symbol(Symbol::new(":source")),
            quoted(Expr::List(vec![
                Expr::Symbol(Symbol::new("file-tail")),
                Expr::Symbol(Symbol::new(":path")),
                Expr::String(path.display().to_string()),
            ])),
            Expr::Symbol(Symbol::new(":decode")),
            Expr::Symbol(Symbol::qualified("test", "decode-record")),
        ],
    )
    .unwrap();

    fs::write(&path, "alpha\n").unwrap();
    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice() == ["alpha".to_owned()]
    });

    fs::write(&path, "").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(&path, "beta\n").unwrap();
    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice()
            == ["alpha".to_owned(), "beta".to_owned()]
    });

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "stop"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", "truncate-server"))],
    )
    .unwrap();
    let _ = fs::remove_file(path);
}

#[test]
fn server_stop_stops_trigger_threads() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "stop-server"), server)
        .unwrap();

    let handle = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "trigger"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "stop-server")),
                Expr::Symbol(Symbol::new(":source")),
                quoted(Expr::Symbol(Symbol::new("stdin"))),
                Expr::Symbol(Symbol::new(":decode")),
                quoted(Expr::Symbol(Symbol::new("lisp"))),
            ],
        )
        .unwrap();
    let trigger = handle.object().downcast_ref::<TriggerHandle>().unwrap();

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "stop"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", "stop-server"))],
    )
    .unwrap();
    wait_until(1_000, || {
        trigger.feed_stdin("gamma\n").is_err() || trigger.is_source_closed()
    });
}

#[test]
fn self_evaluating_trigger_payloads_require_read_eval() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "literal-server"), server)
        .unwrap();

    let handle = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "trigger"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "literal-server")),
                Expr::Symbol(Symbol::new(":source")),
                quoted(Expr::Symbol(Symbol::new("stdin"))),
                Expr::Symbol(Symbol::new(":decode")),
                quoted(Expr::Symbol(Symbol::new("lisp"))),
            ],
        )
        .unwrap();
    let trigger = handle.object().downcast_ref::<TriggerHandle>().unwrap();

    let denied = trigger.inject_text(&mut cx, "\"literal\"\n").unwrap_err();
    assert!(matches!(
        denied,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == read_eval_capability()
    ));

    cx.grant(read_eval_capability());
    assert_eq!(trigger.inject_text(&mut cx, "\"literal\"\n").unwrap(), 1);
}

#[test]
fn webhook_loopback_trigger_uses_queue_source_double() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("webhook-serve");

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

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "webhook-loopback-server"), server)
        .unwrap();

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "trigger"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::qualified("test", "webhook-loopback-server")),
            Expr::Symbol(Symbol::new(":source")),
            quoted(Expr::List(vec![
                Expr::Symbol(Symbol::new("webhook")),
                Expr::Symbol(Symbol::new(":route")),
                Expr::String("/loopback".to_owned()),
                Expr::Symbol(Symbol::new(":loopback")),
                Expr::Bool(true),
            ])),
            Expr::Symbol(Symbol::new(":decode")),
            Expr::Symbol(Symbol::qualified("test", "decode-record")),
        ],
    )
    .unwrap();

    crate::trigger::enqueue_trigger_event(
        &ServerAddress::Webhook {
            route: "/loopback".to_owned(),
        },
        b"queued".to_vec(),
    )
    .unwrap();

    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice() == ["queued".to_owned()]
    });
}

#[cfg(all(feature = "server-net-http", feature = "trigger-webhook"))]
#[test]
fn webhook_trigger_accepts_real_http_posts() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("webhook-serve");

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

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "webhook-server"), server)
        .unwrap();

    let Some(handle) = ({
        let trigger_fn = cx
            .resolve_function(&Symbol::qualified("server", "trigger"))
            .unwrap();
        let args = vec![
            Expr::Symbol(Symbol::qualified("test", "webhook-server")),
            Expr::Symbol(Symbol::new(":source")),
            quoted(Expr::List(vec![
                Expr::Symbol(Symbol::new("webhook")),
                Expr::Symbol(Symbol::new(":route")),
                Expr::String("/hook".to_owned()),
                Expr::Symbol(Symbol::new(":port")),
                Expr::String("0".to_owned()),
            ])),
            Expr::Symbol(Symbol::new(":decode")),
            Expr::Symbol(Symbol::qualified("test", "decode-record")),
        ];
        let mut last_error = None;
        let mut registered_handle = None;
        for _ in 0..10 {
            match cx.call_exprs(trigger_fn.clone(), args.clone()) {
                Ok(value) => {
                    registered_handle = Some(value);
                    break;
                }
                Err(sim_kernel::Error::HostError(message))
                    if message.contains("Operation not permitted") =>
                {
                    last_error = Some(sim_kernel::Error::HostError(message));
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => panic!("unexpected webhook trigger error: {error}"),
            }
        }
        if registered_handle.is_none() {
            eprintln!(
                "skipping real webhook bind in this environment: {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown error".to_owned())
            );
        }
        registered_handle
    }) else {
        return;
    };
    let trigger = handle.object().downcast_ref::<TriggerHandle>().unwrap();
    let port = trigger.webhook_port().unwrap().unwrap();

    let mut stream = PortTcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = b"posted";
    write!(
        stream,
        "POST /hook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();

    wait_until(1_000, || {
        seen.lock().expect("record fn mutex poisoned").as_slice() == ["posted".to_owned()]
    });
}

#[test]
fn server_repl_uses_lisp_for_read_write_and_frame_safe_connection_codec() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let value = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "repl"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new(":codec")),
                Expr::Symbol(Symbol::qualified("codec", "lisp")),
                Expr::Symbol(Symbol::new(":input")),
                Expr::String("42".to_owned()),
                Expr::Symbol(Symbol::new(":output")),
                quoted(Expr::Symbol(Symbol::new("string"))),
            ],
        )
        .unwrap();

    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("42".to_owned())
    );
}
