use super::*;

#[test]
fn all_shared_example_reuses_the_callers_env() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let shared = cx.factory().string("shared".to_owned()).unwrap();
    cx.env_mut().define(Symbol::new("shared"), shared);

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new(":isolate")),
                Expr::Symbol(Symbol::new("all-shared")),
            ],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "shared-server"), server)
        .unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "shared-server"))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "shared-conn"), connection)
        .unwrap();

    let value = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "shared-conn")),
                Expr::Symbol(Symbol::new("shared")),
            ],
        )
        .unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("shared".to_owned())
    );
}

#[test]
fn full_sandbox_example_fails_closed_without_env_or_capability_sharing() {
    let mut cx = strict_name_cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant_named("sandbox.cap");

    let callable = cx
        .factory()
        .opaque(Arc::new(ConstantFn { value: "outside" }))
        .unwrap();
    cx.env_mut().define(Symbol::new("outside"), callable);

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new(":isolate")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new(":env")),
                    Expr::Symbol(Symbol::new("isolate")),
                    Expr::Symbol(Symbol::new(":capabilities")),
                    Expr::Symbol(Symbol::new("isolate")),
                ]),
            ],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "sandbox-server"), server)
        .unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::qualified("test", "sandbox-server"))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "sandbox-conn"), connection)
        .unwrap();

    let missing_symbol = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "sandbox-conn")),
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
                Expr::Symbol(Symbol::qualified("test", "sandbox-conn")),
                Expr::Nil,
                Expr::Symbol(Symbol::new(":requires")),
                Expr::List(vec![Expr::String("sandbox.cap".to_owned())]),
            ],
        )
        .unwrap_err();
    assert!(matches!(
        denied,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == CapabilityName::new("sandbox.cap")
    ));
}

#[test]
fn server_lisp_roundtrips_reflection_except_for_id_and_uptime() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let original = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::Symbol(Symbol::new("local")),
                Expr::Symbol(Symbol::new(":codec")),
                Expr::Symbol(Symbol::qualified("codec", "json")),
                Expr::Symbol(Symbol::new(":name")),
                Expr::Symbol(Symbol::qualified("demo", "srv")),
                Expr::Symbol(Symbol::new(":isolate")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new(":env")),
                    Expr::Symbol(Symbol::new("child")),
                ]),
            ],
        })
        .unwrap();

    let lisp = cx
        .call_function(
            &Symbol::qualified("server", "lisp"),
            sim_kernel::Args::new(vec![original.clone()]),
        )
        .unwrap();
    let lisp_expr = lisp.object().as_expr(&mut cx).unwrap();
    let rebuilt = cx.eval_expr(lisp_expr).unwrap();

    let original_reflect = cx
        .call_function(
            &Symbol::qualified("server", "reflect"),
            sim_kernel::Args::new(vec![original]),
        )
        .unwrap();
    let rebuilt_reflect = cx
        .call_function(
            &Symbol::qualified("server", "reflect"),
            sim_kernel::Args::new(vec![rebuilt]),
        )
        .unwrap();

    let original_map = normalized_reflect_table(&mut cx, original_reflect);
    let rebuilt_map = normalized_reflect_table(&mut cx, rebuilt_reflect);
    assert_eq!(original_map, rebuilt_map);
}
