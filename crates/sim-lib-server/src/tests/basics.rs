use super::*;
use sim_kernel::{Args, Cx, ObjectCompat, Value};

#[test]
fn server_frame_payloads_roundtrip_through_installed_codecs() {
    let mut cx = cx();
    let expr = Expr::Call {
        operator: Box::new(Expr::Symbol(Symbol::new("echo"))),
        args: vec![Expr::String("wire".to_owned()), Expr::Bool(true)],
    };

    for codec in installed_codecs() {
        let frame = ServerFrame::from_expr(
            &mut cx,
            codec.clone(),
            FrameKind::Request,
            &expr,
            Consistency::RemoteOnly,
            vec![
                CapabilityName::new("server.use"),
                CapabilityName::new("codec.decode"),
            ],
            true,
        )
        .unwrap();

        let decoded = frame.decode_expr(&mut cx, ReadPolicy::default()).unwrap();
        assert_eq!(decoded, expr);
        assert_eq!(frame.codec, codec);
        assert_eq!(frame.envelope.consistency, Consistency::RemoteOnly);
        assert_eq!(
            frame.envelope.required_capabilities,
            vec![
                CapabilityName::new("server.use"),
                CapabilityName::new("codec.decode"),
            ]
        );
        assert!(frame.envelope.trace);
    }
}

#[test]
fn server_runtime_objects_are_browseable() {
    let mut cx = cx();
    let codecs = installed_codecs();
    let default_codec = codecs[0].clone();
    let site = Arc::new(LoopbackSite {
        address: ServerAddress::InProcess { thread: 1 },
        codecs: codecs.clone(),
    });

    let server = Server::new(
        ServerAddress::Local,
        default_codec.clone(),
        codecs.clone(),
        crate::ThreadMode::Coop,
        IsolationPolicy::default(),
        None,
        site.clone(),
        vec![(Symbol::new("address"), Expr::Symbol(Symbol::new("local")))],
    )
    .unwrap();
    let connection = Connection::new(
        ServerAddress::InProcess { thread: 1 },
        default_codec.clone(),
        codecs.clone(),
        site,
    )
    .unwrap();
    let frame = ServerFrame::from_expr(
        &mut cx,
        default_codec,
        FrameKind::Notify,
        &Expr::String("payload".to_owned()),
        Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap();

    let server_table = server.as_table(&mut cx).unwrap();
    let connection_table = connection.as_table(&mut cx).unwrap();
    let frame_table = frame.as_table(&mut cx).unwrap();
    let Expr::Map(server_entries) = server_table.object().as_expr(&mut cx).unwrap() else {
        panic!("server table should encode as a map");
    };

    assert!(matches!(
        connection_table.object().as_expr(&mut cx).unwrap(),
        Expr::Map(_)
    ));
    assert!(matches!(
        frame_table.object().as_expr(&mut cx).unwrap(),
        Expr::Map(_)
    ));
    assert_eq!(server.display(&mut cx).unwrap(), "#<server>");
    assert_eq!(
        table_field(&server_entries, "listening"),
        Some(Expr::Bool(false))
    );
    assert_eq!(
        table_field(&server_entries, "sessions"),
        Some(Expr::String("0".to_owned()))
    );
    assert_eq!(connection.display(&mut cx).unwrap(), "#<server-connection>");
    assert!(frame.display(&mut cx).unwrap().contains("notify"));
}

#[test]
fn server_helpers_use_codec_registry_instead_of_parallel_serialization() {
    let mut cx = cx();
    let codec = installed_codecs()[0].clone();
    let expr = Expr::String("registry".to_owned());

    let payload =
        encode_frame_payload(&mut cx, &codec, &expr, sim_kernel::EncodeOptions::default()).unwrap();
    let decoded = decode_frame_payload(
        &mut cx,
        &codec,
        &payload,
        ReadPolicy::default(),
        Default::default(),
    )
    .unwrap();

    assert_eq!(decoded, expr);
    assert!(cx.registry().codec_by_symbol(&codec).is_some());
}

#[test]
fn connection_realize_requires_eval_remote_for_remote_like_addresses() {
    let mut cx = cx();
    let codecs = installed_codecs();
    let connection = Connection::new(
        ServerAddress::InProcess { thread: 7 },
        codecs[0].clone(),
        codecs.clone(),
        Arc::new(ResponseSite {
            address: ServerAddress::Local,
            codecs,
        }),
    )
    .unwrap();

    let request = EvalRequest {
        expr: Expr::Nil,
        mode: sim_kernel::EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::LocalFirst,
        trace: false,
    };
    let denied = connection.realize(&mut cx, request.clone());
    assert!(matches!(
        denied,
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));

    cx.grant(eval_remote_capability());
    let reply = connection.realize(&mut cx, request).unwrap();
    assert!(matches!(
        reply.value.object().as_expr(&mut cx).unwrap(),
        Expr::Nil
    ));
}

#[test]
fn install_server_lib_is_idempotent() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    install_server_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&Symbol::new("server")).is_some());
}

#[test]
fn every_exported_server_symbol_resolves_to_a_callable() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let manifest = &cx.registry().lib(&Symbol::new("server")).unwrap().manifest;
    for export in manifest.exports.clone() {
        let sim_kernel::Export::Function { symbol, .. } = export else {
            continue;
        };
        let value = cx.resolve_function(&symbol).unwrap();
        assert!(
            value.object().as_callable().is_some(),
            "{symbol} should resolve to a callable"
        );
    }
}

#[test]
fn server_lib_claims_loaded_cli_server_entrypoint() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    let symbol = Symbol::qualified("cli", "main/server");
    let envelope = cli_envelope(&mut cx, "server", &["server", "--dry-run"]);

    let value = cx
        .call_function(&symbol, Args::new(vec![envelope]))
        .unwrap();

    assert!(value.object().truth(&mut cx).unwrap());
}

#[test]
fn server_connect_supports_server_values_and_local_addresses() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "server"), server.clone())
        .unwrap();

    let from_server = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "server"))],
        )
        .unwrap();
    let server_connection = from_server.object().downcast_ref::<Connection>().unwrap();
    assert_eq!(server_connection.address(), &ServerAddress::Local);

    let from_local = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::Symbol(Symbol::new("local")))],
        )
        .unwrap();
    let local_connection = from_local.object().downcast_ref::<Connection>().unwrap();
    assert_eq!(local_connection.address(), &ServerAddress::Local);
}

fn cli_envelope(cx: &mut Cx, verb: &str, args: &[&str]) -> Value {
    let verb = cx.factory().string(verb.to_owned()).unwrap();
    let args = cx
        .factory()
        .list(
            args.iter()
                .map(|arg| cx.factory().string((*arg).to_owned()).unwrap())
                .collect(),
        )
        .unwrap();
    cx.factory()
        .table(vec![
            (Symbol::new("verb"), verb),
            (Symbol::new("args"), args),
        ])
        .unwrap()
}

#[test]
fn server_connect_resolves_pipeline_addresses_to_single_connections() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let pipeline = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::List(vec![
                Expr::Symbol(Symbol::new("pipeline")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new("local")),
            ]))],
        )
        .unwrap();
    let connection = pipeline.object().downcast_ref::<Connection>().unwrap();
    assert!(matches!(
        connection.address(),
        ServerAddress::Pipeline { steps } if steps.len() == 2
    ));
}

#[test]
fn parses_full_address_and_isolation_surfaces() {
    let address = ServerAddress::from_expr(&Expr::List(vec![
        Expr::Symbol(Symbol::new("tcp")),
        Expr::Symbol(Symbol::new(":host")),
        Expr::String("0.0.0.0".to_owned()),
        Expr::Symbol(Symbol::new(":port")),
        Expr::String("5555".to_owned()),
    ]))
    .unwrap();
    assert_eq!(
        address,
        ServerAddress::Tcp {
            host: "0.0.0.0".to_owned(),
            port: 5555,
        }
    );

    let isolation = IsolationPolicy::from_expr(&Expr::List(vec![
        Expr::Symbol(Symbol::new(":env")),
        Expr::Symbol(Symbol::new("child")),
        Expr::Symbol(Symbol::new(":libs")),
        Expr::Symbol(Symbol::new("share")),
        Expr::Symbol(Symbol::new(":factory")),
        Expr::Symbol(Symbol::new("isolate")),
        Expr::Symbol(Symbol::new(":registry")),
        Expr::List(vec![
            Expr::Symbol(Symbol::new("import")),
            Expr::Symbol(Symbol::qualified("snap", "registry")),
        ]),
        Expr::Symbol(Symbol::new(":capabilities")),
        Expr::Symbol(Symbol::new("share")),
    ]))
    .unwrap();
    assert!(matches!(isolation.env, crate::ShareMode::Child));
    assert!(matches!(isolation.factory, crate::ShareMode::Isolate));
    assert!(matches!(
        isolation.registry,
        crate::ShareMode::Import(symbol) if symbol == Symbol::qualified("snap", "registry")
    ));
}
