use super::*;

#[test]
fn all_shared_example_reuses_the_callers_env_over_tcp() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant(eval_remote_capability());
    cx.grant_named("network");

    let shared = cx.factory().string("shared".to_owned()).unwrap();
    cx.env_mut().define(Symbol::new("shared"), shared);

    let server = match cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "start"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":address")),
            tcp_loopback_expr(),
            Expr::Symbol(Symbol::new(":isolate")),
            Expr::Symbol(Symbol::new("all-shared")),
        ],
    ) {
        Ok(server) => server,
        Err(sim_kernel::Error::HostError(message)) if message.contains("PermissionDenied") => {
            return;
        }
        Err(error) => panic!("tcp server start failed: {error}"),
    };
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "shared-tcp-server"),
            server.clone(),
        )
        .unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "shared-tcp-server"))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "shared-tcp-conn"), connection)
        .unwrap();
    let value = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "shared-tcp-conn")),
                Expr::Symbol(Symbol::new("shared")),
            ],
        )
        .unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("shared".to_owned())
    );
    stop_server(&mut cx, server);
}

#[test]
fn full_sandbox_example_fails_closed_without_env_or_capability_sharing_over_tcp() {
    let mut cx = strict_name_cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant(eval_remote_capability());
    cx.grant_named("network");
    cx.grant_named("sandbox.cap");

    let callable = cx
        .factory()
        .opaque(Arc::new(ConstantFn { value: "outside" }))
        .unwrap();
    cx.env_mut().define(Symbol::new("outside"), callable);

    let server = match cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "start"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":address")),
            tcp_loopback_expr(),
            Expr::Symbol(Symbol::new(":isolate")),
            Expr::List(vec![
                Expr::Symbol(Symbol::new(":env")),
                Expr::Symbol(Symbol::new("isolate")),
                Expr::Symbol(Symbol::new(":capabilities")),
                Expr::Symbol(Symbol::new("isolate")),
            ]),
        ],
    ) {
        Ok(server) => server,
        Err(sim_kernel::Error::HostError(message)) if message.contains("PermissionDenied") => {
            return;
        }
        Err(error) => panic!("tcp server start failed: {error}"),
    };
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "sandbox-tcp-server"),
            server.clone(),
        )
        .unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified(
                "test",
                "sandbox-tcp-server",
            ))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "sandbox-tcp-conn"),
            connection.clone(),
        )
        .unwrap();
    let missing_symbol = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "sandbox-tcp-conn")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::new("outside"))),
                    args: Vec::new(),
                },
            ],
        )
        .unwrap_err();
    assert!(!format!("{missing_symbol}").is_empty());

    let denied = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "sandbox-tcp-conn")),
                Expr::Nil,
                Expr::Symbol(Symbol::new(":requires")),
                Expr::List(vec![Expr::String("sandbox.cap".to_owned())]),
            ],
        )
        .unwrap_err();
    assert!(!format!("{denied}").is_empty());
    stop_server(&mut cx, server);
}

fn stop_server(cx: &mut sim_kernel::Cx, server: sim_kernel::Value) {
    cx.call_function(
        &Symbol::qualified("server", "stop"),
        sim_kernel::Args::new(vec![server]),
    )
    .unwrap();
}

fn tcp_loopback_expr() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::new("tcp")),
        Expr::Symbol(Symbol::new(":host")),
        Expr::String("127.0.0.1".to_owned()),
        Expr::Symbol(Symbol::new(":port")),
        Expr::String("0".to_owned()),
    ])
}
