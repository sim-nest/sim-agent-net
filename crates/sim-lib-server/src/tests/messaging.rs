use super::*;

#[test]
fn server_pipeline_threads_requests_left_to_right_and_stamps_hops() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let codecs = installed_codecs();
    let codec = Symbol::qualified("codec", "json");
    let hops_a = Arc::new(Mutex::new(Vec::new()));
    let hops_b = Arc::new(Mutex::new(Vec::new()));
    let conn_a = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                codec.clone(),
                codecs.clone(),
                Arc::new(TransformSite {
                    address: ServerAddress::Local,
                    codecs: codecs.clone(),
                    prefix: "a:",
                    hops: hops_a.clone(),
                }),
            )
            .unwrap(),
        ))
        .unwrap();
    let conn_b = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                codec.clone(),
                codecs.clone(),
                Arc::new(TransformSite {
                    address: ServerAddress::Local,
                    codecs: codecs.clone(),
                    prefix: "b:",
                    hops: hops_b.clone(),
                }),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "pipe-a"), conn_a)
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "pipe-b"), conn_b)
        .unwrap();

    let pipeline = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "pipeline"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::new(":steps")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::new("list"))),
                    args: vec![
                        Expr::Symbol(Symbol::qualified("test", "pipe-a")),
                        Expr::Symbol(Symbol::qualified("test", "pipe-b")),
                    ],
                },
            ],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "pipeline"), pipeline)
        .unwrap();

    let result = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "pipeline")),
                Expr::String("seed".to_owned()),
            ],
        )
        .unwrap();

    assert_eq!(
        result.object().as_expr(&mut cx).unwrap(),
        Expr::String("b:a:seed".to_owned())
    );
    assert_eq!(*hops_a.lock().expect("hop-a mutex poisoned"), vec![1]);
    assert_eq!(*hops_b.lock().expect("hop-b mutex poisoned"), vec![2]);
}

#[test]
fn server_loop_repeats_until_predicate_fires_and_start_loop_accepts_it() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let codecs = installed_codecs();
    let codec = Symbol::qualified("codec", "json");
    let conn = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                codec.clone(),
                codecs.clone(),
                Arc::new(TransformSite {
                    address: ServerAddress::Local,
                    codecs: codecs.clone(),
                    prefix: "x",
                    hops: Arc::new(Mutex::new(Vec::new())),
                }),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "loop-step"), conn)
        .unwrap();
    let until = cx
        .factory()
        .opaque(Arc::new(UntilValueFn { expected: "xxx" }))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "until-xxx"), until)
        .unwrap();

    let loop_value = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "loop"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::new(":steps")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::new("list"))),
                    args: vec![Expr::Symbol(Symbol::qualified("test", "loop-step"))],
                },
                Expr::Symbol(Symbol::new(":max-iterations")),
                Expr::String("5".to_owned()),
                Expr::Symbol(Symbol::new(":until")),
                Expr::Symbol(Symbol::qualified("test", "until-xxx")),
            ],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "loop"), loop_value.clone())
        .unwrap();

    let started = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "start-loop"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "loop"))],
        )
        .unwrap();
    assert!(started.object().downcast_ref::<Connection>().is_some());

    let result = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "loop")),
                Expr::String(String::new()),
            ],
        )
        .unwrap();
    assert_eq!(
        result.object().as_expr(&mut cx).unwrap(),
        Expr::String("xxx".to_owned())
    );
}

#[test]
fn messaging_surface_and_realize_wrapper_work() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant(eval_remote_capability());

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::Symbol(Symbol::new("local")))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "conn"), connection.clone())
        .unwrap();

    let requested = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "conn")),
                Expr::String("request-ok".to_owned()),
            ],
        )
        .unwrap();
    assert_eq!(
        requested.object().as_expr(&mut cx).unwrap(),
        Expr::String("request-ok".to_owned())
    );

    let sent = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "send"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "conn")),
                Expr::String("send-ok".to_owned()),
            ],
        )
        .unwrap();
    let Expr::String(msg_id) = sent.object().as_expr(&mut cx).unwrap() else {
        panic!("server/send should return a message id string");
    };

    let received = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "receive"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "conn"))],
        )
        .unwrap();
    let frame = received.object().downcast_ref::<ServerFrame>().unwrap();
    assert_eq!(frame.kind, FrameKind::Response);
    assert_eq!(frame.correlate, Some(msg_id.parse::<u64>().unwrap()));

    let as_fabric = cx
        .call_function(
            &Symbol::qualified("server", "as-fabric"),
            sim_kernel::Args::new(vec![connection]),
        )
        .unwrap();
    assert!(as_fabric.object().as_eval_fabric().is_some());

    let realized = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "realize"))
                .unwrap(),
            vec![
                Expr::String("realize-ok".to_owned()),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Symbol(Symbol::qualified("test", "conn")),
            ],
        )
        .unwrap();
    assert_eq!(
        realized.object().as_expr(&mut cx).unwrap(),
        Expr::String("realize-ok".to_owned())
    );
}

#[test]
fn server_notify_builds_notify_frames() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let seen = Arc::new(Mutex::new(Vec::new()));
    let codecs = installed_codecs();
    let connection = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                codecs[0].clone(),
                codecs.clone(),
                Arc::new(RecordingSite {
                    address: ServerAddress::Local,
                    codecs,
                    seen: seen.clone(),
                }),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "notify-conn"), connection)
        .unwrap();

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "notify"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::qualified("test", "notify-conn")),
            Expr::String("notify-ok".to_owned()),
        ],
    )
    .unwrap();

    assert_eq!(
        seen.lock()
            .expect("recording site mutex poisoned")
            .as_slice(),
        &[FrameKind::Notify]
    );
}

#[test]
fn server_request_and_send_honor_explicit_consistency_overrides() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let local = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::Symbol(Symbol::new("local")))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "local-conn"), local)
        .unwrap();

    let remote = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Tcp {
                    host: "127.0.0.1".to_owned(),
                    port: 9,
                },
                Symbol::qualified("codec", "lisp"),
                installed_codecs(),
                Arc::new(crate::LocalEvalSite::new(
                    ServerAddress::Tcp {
                        host: "127.0.0.1".to_owned(),
                        port: 9,
                    },
                    installed_codecs(),
                )),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "remote-conn"), remote)
        .unwrap();

    let local_request_err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "local-conn")),
                Expr::String("x".to_owned()),
                Expr::Symbol(Symbol::new(":consistency")),
                Expr::Symbol(Symbol::new("remote-only")),
            ],
        )
        .unwrap_err();
    assert!(format!("{local_request_err}").contains("remote-only refuses local"));

    let remote_request_err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "remote-conn")),
                Expr::String("x".to_owned()),
                Expr::Symbol(Symbol::new(":consistency")),
                Expr::Symbol(Symbol::new("local-only")),
            ],
        )
        .unwrap_err();
    assert!(format!("{remote_request_err}").contains("local-only refuses remote"));

    let local_send_err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "send"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "local-conn")),
                Expr::String("x".to_owned()),
                Expr::Symbol(Symbol::new(":consistency")),
                Expr::Symbol(Symbol::new("remote-only")),
            ],
        )
        .unwrap_err();
    assert!(format!("{local_send_err}").contains("remote-only refuses local"));

    let remote_send_err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "send"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "remote-conn")),
                Expr::String("x".to_owned()),
                Expr::Symbol(Symbol::new(":consistency")),
                Expr::Symbol(Symbol::new("local-only")),
            ],
        )
        .unwrap_err();
    assert!(format!("{remote_send_err}").contains("local-only refuses remote"));
}

#[test]
fn streaming_surface_buffers_chunks_and_marks_done() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let codecs = installed_codecs();
    let connection = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                codecs[0].clone(),
                codecs.clone(),
                Arc::new(MultiChunkSite {
                    address: ServerAddress::Local,
                    codecs,
                }),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "stream-conn"), connection)
        .unwrap();

    let handle = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "stream"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "stream-conn")),
                Expr::String("stream-me".to_owned()),
            ],
        )
        .unwrap();
    let stream = handle.object().downcast_ref::<StreamHandle>().unwrap();
    assert!(stream.is_done());
    assert_eq!(stream.buffered_len(), 2);

    cx.registry_mut()
        .register_value(Symbol::qualified("test", "stream"), handle.clone())
        .unwrap();

    let alpha = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "stream-next"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "stream"))],
        )
        .unwrap();
    let beta = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "stream-next"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "stream"))],
        )
        .unwrap();
    let end = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "stream-next"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "stream"))],
        )
        .unwrap();
    let done = cx
        .call_function(
            &Symbol::qualified("server", "stream-done?"),
            sim_kernel::Args::new(vec![handle.clone()]),
        )
        .unwrap();

    assert_eq!(
        alpha.object().as_expr(&mut cx).unwrap(),
        Expr::String("alpha".to_owned())
    );
    assert_eq!(
        beta.object().as_expr(&mut cx).unwrap(),
        Expr::String("beta".to_owned())
    );
    assert!(matches!(end.object().as_expr(&mut cx).unwrap(), Expr::Nil));
    assert!(matches!(
        done.object().as_expr(&mut cx).unwrap(),
        Expr::Bool(true)
    ));

    let cancelled = cx
        .call_function(
            &Symbol::qualified("server", "stream-cancel"),
            sim_kernel::Args::new(vec![handle.clone()]),
        )
        .unwrap();
    assert!(matches!(
        cancelled.object().as_expr(&mut cx).unwrap(),
        Expr::Nil
    ));
    assert!(stream.is_cancelled());
}
