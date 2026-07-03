use super::*;

#[test]
fn lifecycle_functions_change_and_report_status() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let started = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new(":name")),
                Expr::Symbol(Symbol::qualified("demo", "srv")),
            ],
        })
        .unwrap();
    let server = started.object().downcast_ref::<Server>().unwrap();
    assert_eq!(server.status(), ServerStatus::Running);

    let suspended = cx
        .call_function(
            &Symbol::qualified("server", "suspend"),
            sim_kernel::Args::new(vec![started.clone()]),
        )
        .unwrap();
    assert_eq!(
        suspended
            .object()
            .downcast_ref::<Server>()
            .unwrap()
            .status(),
        ServerStatus::Suspended
    );

    let resumed = cx
        .call_function(
            &Symbol::qualified("server", "resume"),
            sim_kernel::Args::new(vec![started.clone()]),
        )
        .unwrap();
    assert_eq!(
        resumed.object().downcast_ref::<Server>().unwrap().status(),
        ServerStatus::Running
    );

    let stopped = cx
        .call_function(
            &Symbol::qualified("server", "stop"),
            sim_kernel::Args::new(vec![started.clone()]),
        )
        .unwrap();
    assert_eq!(
        stopped.object().downcast_ref::<Server>().unwrap().status(),
        ServerStatus::Stopped
    );

    let health = cx
        .call_function(
            &Symbol::qualified("server", "health"),
            sim_kernel::Args::new(vec![started]),
        )
        .unwrap();
    let Expr::Map(entries) = health.object().as_expr(&mut cx).unwrap() else {
        panic!("server/health should produce a table expression");
    };
    assert_eq!(
        table_field(&entries, "status"),
        Some(Expr::Symbol(Symbol::new("stopped")))
    );
    assert_eq!(
        table_field(&entries, "sessions"),
        Some(Expr::String("0".to_owned()))
    );
    assert_eq!(table_field(&entries, "listening"), None);
}

#[test]
fn coroutine_servers_yield_resume_and_cancel() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant(eval_remote_capability());

    let site = cx.factory().opaque(Arc::new(YieldingSiteFn)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "yield-site"), site)
        .unwrap();

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::List(vec![Expr::Symbol(Symbol::new("coroutine"))]),
                Expr::Symbol(Symbol::new(":thread")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("coroutine")),
                    Expr::Symbol(Symbol::new("main")),
                ]),
                Expr::Symbol(Symbol::new(":site")),
                Expr::Symbol(Symbol::qualified("test", "yield-site")),
            ],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "coro-server"), server.clone())
        .unwrap();

    let status = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "coroutine-status"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "coro-server"))],
        )
        .unwrap();
    assert_eq!(
        status.object().as_expr(&mut cx).unwrap(),
        Expr::Symbol(Symbol::new("suspended"))
    );

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "coro-server"))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "coro-conn"), connection)
        .unwrap();

    let yielded = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "coro-conn")),
                Expr::String("first".to_owned()),
            ],
        )
        .unwrap();
    assert_eq!(
        yielded.object().as_expr(&mut cx).unwrap(),
        Expr::String("first".to_owned())
    );

    let resumed = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "resume"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "coro-server")),
                Expr::String("second".to_owned()),
            ],
        )
        .unwrap();
    assert_eq!(
        resumed.object().as_expr(&mut cx).unwrap(),
        Expr::String("second".to_owned())
    );

    let lisp = cx
        .call_function(
            &Symbol::qualified("server", "lisp"),
            sim_kernel::Args::new(vec![server.clone()]),
        )
        .unwrap();
    let Expr::Call { args, .. } = lisp.object().as_expr(&mut cx).unwrap() else {
        panic!("server/lisp should return a call expression");
    };
    assert!(args.windows(2).any(|pair| {
        pair[0] == Expr::Symbol(Symbol::new(":thread"))
            && pair[1]
                == Expr::List(vec![
                    Expr::Symbol(Symbol::new("coroutine")),
                    Expr::Symbol(Symbol::new("main")),
                ])
    }));

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "cancel-coroutine"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", "coro-server"))],
    )
    .unwrap();

    let done = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "coroutine-status"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "coro-server"))],
        )
        .unwrap();
    assert_eq!(
        done.object().as_expr(&mut cx).unwrap(),
        Expr::Symbol(Symbol::new("done"))
    );
}

#[test]
fn spawn_and_pool_thread_modes_start_cleanly() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    for thread in ["spawn", "pool"] {
        let server = cx
            .eval_expr(Expr::Call {
                operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
                args: vec![
                    Expr::Symbol(Symbol::new(":thread")),
                    Expr::Symbol(Symbol::new(thread)),
                ],
            })
            .unwrap();
        let reflected = cx
            .call_function(
                &Symbol::qualified("server", "reflect"),
                sim_kernel::Args::new(vec![server.clone()]),
            )
            .unwrap();
        let Expr::Map(entries) = reflected.object().as_expr(&mut cx).unwrap() else {
            panic!("server/reflect should produce a table expression");
        };
        assert_eq!(
            table_field(&entries, "thread"),
            Some(Expr::Symbol(Symbol::new(thread)))
        );
        let _ = cx
            .call_function(
                &Symbol::qualified("server", "stop"),
                sim_kernel::Args::new(vec![server]),
            )
            .unwrap();
    }
}

#[test]
fn unsupported_coroutine_thread_base_reports_current_error() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let err = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":thread")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("coroutine")),
                    Expr::Symbol(Symbol::new("spawn")),
                ]),
            ],
        })
        .unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::Eval(message)
            if message == "server/start: coroutine thread mode supports only main or coop base"
    ));
}

#[test]
fn server_health_reports_live_runtime_counters() {
    let mut eval_cx = cx();
    let codecs = installed_codecs();
    let default_codec = codecs[0].clone();
    let site = Arc::new(LoopbackSite {
        address: ServerAddress::InProcess { thread: 7 },
        codecs: codecs.clone(),
    });
    let transport = Arc::new(crate::transport::RegistryTransport::new(
        ServerAddress::InProcess { thread: 7 },
    ));
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        crate::ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let session_id = runtime
        .open_session(
            Symbol::qualified("codec", "binary"),
            IsolationPolicy::default(),
        )
        .unwrap();
    runtime.note_message_received();
    runtime.note_message_sent();
    runtime
        .update_session_codec(session_id, Symbol::qualified("codec", "json"))
        .unwrap();
    let server = Server::with_runtime(
        ServerAddress::InProcess { thread: 7 },
        default_codec,
        codecs,
        crate::ThreadMode::Spawn,
        IsolationPolicy::default(),
        None,
        site,
        vec![],
        Some(runtime),
    )
    .unwrap();

    let health = server.health_value(&mut eval_cx).unwrap();
    let Expr::Map(entries) = health.object().as_expr(&mut eval_cx).unwrap() else {
        panic!("server/health should produce a table expression");
    };
    assert_eq!(
        table_field(&entries, "sessions"),
        Some(Expr::String("1".to_owned()))
    );
    assert_eq!(
        table_field(&entries, "connections"),
        Some(Expr::String("1".to_owned()))
    );
    assert_eq!(
        table_field(&entries, "messages-sent"),
        Some(Expr::String("1".to_owned()))
    );
    assert_eq!(
        table_field(&entries, "messages-received"),
        Some(Expr::String("1".to_owned()))
    );
}

#[test]
fn server_sessions_lists_runtime_session_tables() {
    let mut eval_cx = cx();
    let codecs = installed_codecs();
    let site = Arc::new(LoopbackSite {
        address: ServerAddress::InProcess { thread: 9 },
        codecs: codecs.clone(),
    });
    let transport = Arc::new(crate::transport::RegistryTransport::new(
        ServerAddress::InProcess { thread: 9 },
    ));
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        crate::ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    runtime
        .open_session(
            Symbol::qualified("codec", "binary"),
            IsolationPolicy::default(),
        )
        .unwrap();
    let server = Server::with_runtime(
        ServerAddress::InProcess { thread: 9 },
        codecs[0].clone(),
        codecs,
        crate::ThreadMode::Spawn,
        IsolationPolicy::default(),
        None,
        site,
        vec![],
        Some(runtime),
    )
    .unwrap();

    let sessions = server.sessions_value(&mut eval_cx).unwrap();
    let Expr::List(items) = sessions.object().as_expr(&mut eval_cx).unwrap() else {
        panic!("server/sessions should return a list");
    };
    assert_eq!(items.len(), 1);
    let Expr::Map(entries) = items[0].clone() else {
        panic!("server/sessions entries should be tables");
    };
    assert_eq!(
        table_field(&entries, "negotiated-codec"),
        Some(Expr::Symbol(Symbol::qualified("codec", "binary")))
    );
    assert_eq!(table_field(&entries, "closed"), Some(Expr::Bool(false)));
}

#[test]
fn server_close_marks_connection_session_closed() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::Symbol(Symbol::new("local")))],
        )
        .unwrap();
    let connection_ref = connection.object().downcast_ref::<Connection>().unwrap();
    assert!(!connection_ref.session().closed);

    let closed = cx
        .call_function(
            &Symbol::qualified("server", "close"),
            sim_kernel::Args::new(vec![connection.clone()]),
        )
        .unwrap();
    let closed_ref = closed.object().downcast_ref::<Connection>().unwrap();
    assert!(closed_ref.session().closed);
}
