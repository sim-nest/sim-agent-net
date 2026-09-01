#[test]
fn parent_conduct_delegates_debate_and_market_under_one_child_budget() {
    let mut cx = topology_cx();
    let judge = rubric_judge(&mut cx, "pro wins evidence");
    register_connection(
        &mut cx,
        Symbol::qualified("test", "debate-pro"),
        fixed_reply_connection("pro wins evidence"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "debate-con"),
        fixed_reply_connection("con loses"),
    );
    let debate = rust_debate(&mut cx, judge);
    let _worker = register_bid_worker(
        &mut cx,
        Symbol::qualified("test", "nested-market-worker"),
        1.0,
        "market typed result",
    );
    let router = bid_router(&mut cx);
    let market_value = cx
        .resolve_value(&Symbol::qualified("test", "nested-market-worker"))
        .unwrap();
    let market = build_market_data_graph_connection(&mut cx, vec![market_value], router).unwrap();

    let unit = Symbol::qualified("agent.usage", "child-run");
    let charge = UsageQuantity {
        unit: unit.clone(),
        amount: 1,
    };
    let budget = AgentUsageBudget::new(vec![UsageQuantity {
        unit: unit.clone(),
        amount: 2,
    }])
    .unwrap();
    let mut parent = AgentRunFrame::standard(
        Symbol::qualified("run", "parent-conduct"),
        Expr::String("choose".into()),
    );
    let allowed = CapabilitySet::new().grant(topology_run_capability());
    let debate_result = execute_delegate_once(
        &mut cx,
        &mut parent,
        &debate,
        &budget,
        DelegateRequest {
            correlation: Symbol::qualified("child", "debate"),
            allowed: allowed.clone(),
            charge: charge.clone(),
            request: request(Expr::String("topic".into())),
        },
    )
    .unwrap();
    let market_result = execute_delegate_once(
        &mut cx,
        &mut parent,
        market.as_ref(),
        &budget,
        DelegateRequest {
            correlation: Symbol::qualified("child", "market"),
            allowed,
            charge,
            request: request(Expr::String("route".into())),
        },
    )
    .unwrap();

    assert_ne!(map_expr_field(&debate_result.output, "winner"), Expr::Nil);
    assert_ne!(map_expr_field(&debate_result.output, "verdict"), Expr::Nil);
    assert_ne!(map_expr_field(&market_result.output, "winner"), Expr::Nil);
    assert_eq!(parent.usage.amount(&unit), 2);
    assert_eq!(
        debate_result.correlation,
        Symbol::qualified("child", "debate")
    );
    assert_eq!(
        market_result.correlation,
        Symbol::qualified("child", "market")
    );
}

fn request(expr: Expr) -> EvalRequest {
    EvalRequest {
        expr,
        mode: EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream: false,
        stream_buffer: None,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::LocalFirst,
        trace: false,
    }
}

fn topology_cx() -> Cx {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant(topology_run_capability());
    cx
}

fn request_expr(cx: &mut Cx, connection: &Connection, input: Expr) -> Expr {
    connection
        .request(cx, input, None, Vec::new())
        .unwrap()
        .object()
        .as_expr(cx)
        .unwrap()
}

fn connection_value(value: &sim_kernel::Value) -> Connection {
    value.object().downcast_ref::<Connection>().unwrap().clone()
}

fn rust_ring(cx: &mut Cx) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "ring"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":agents")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "ring-a")),
                        Expr::Symbol(Symbol::qualified("test", "ring-b")),
                        Expr::Symbol(Symbol::qualified("test", "ring-c")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":role-cycle")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::new("planner")),
                        Expr::Symbol(Symbol::new("worker")),
                        Expr::Symbol(Symbol::new("critic")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":max-turns")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "3".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_star(cx: &mut Cx) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "star"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":hub")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "hub")).unwrap(),
                cx.factory().symbol(Symbol::new(":spokes")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "spoke-b")),
                        Expr::Symbol(Symbol::qualified("test", "spoke-c")),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_mesh(cx: &mut Cx, judge: sim_kernel::Value) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "mesh"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":agents")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "mesh-a")),
                        Expr::Symbol(Symbol::qualified("test", "mesh-b")),
                        Expr::Symbol(Symbol::qualified("test", "mesh-c")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                judge,
                cx.factory().symbol(Symbol::new(":max-rounds")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "2".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_market(cx: &mut Cx, router: sim_kernel::Value) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "market"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":workers")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "expensive")),
                        Expr::Symbol(Symbol::qualified("test", "cheap")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":router")).unwrap(),
                router,
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_debate(cx: &mut Cx, judge: sim_kernel::Value) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "debate"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":pro")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "debate-pro"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":con")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "debate-con"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                judge,
                cx.factory().symbol(Symbol::new(":rounds")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "1".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_speculate_verify(cx: &mut Cx) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "speculate-verify"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":speculator")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "spec-fast"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":verifier")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "verify-different"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":on-mismatch")).unwrap(),
                cx.factory().symbol(Symbol::new("retry")).unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rust_open_claw(cx: &mut Cx) -> Connection {
    let value = cx
        .call_function(
            &Symbol::qualified("topology", "open-claw"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":steps")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "open-a")),
                        Expr::Symbol(Symbol::qualified("test", "open-b")),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    connection_value(&value)
}

fn rubric_judge(cx: &mut Cx, reference: &str) -> sim_kernel::Value {
    cx.call_function(
        &Symbol::qualified("judge", "rubric"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":reference")).unwrap(),
            cx.factory().string(reference.to_owned()).unwrap(),
        ]),
    )
    .unwrap()
}

fn bid_router(cx: &mut Cx) -> sim_kernel::Value {
    cx.call_function(
        &Symbol::qualified("router", "bid"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":targets")).unwrap(),
            cx.factory()
                .expr(Expr::List(vec![
                    Expr::Symbol(Symbol::qualified("test", "expensive")),
                    Expr::Symbol(Symbol::qualified("test", "cheap")),
                ]))
                .unwrap(),
        ]),
    )
    .unwrap()
}
