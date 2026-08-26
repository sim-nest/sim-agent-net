#[test]
fn public_ring_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "ring-a"),
        tagged_append_connection("a"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "ring-b"),
        tagged_append_connection("b"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "ring-c"),
        tagged_append_connection("c"),
    );
    let agents = vec![
        cx.resolve_value(&Symbol::qualified("test", "ring-a"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "ring-b"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "ring-c"))
            .unwrap(),
    ];
    let roles = vec![
        Symbol::new("planner"),
        Symbol::new("worker"),
        Symbol::new("critic"),
    ];

    let rust = rust_ring(&mut cx);
    let data = build_ring_data_graph_connection(&mut cx, agents, roles, 3).unwrap();
    let input = Expr::List(Vec::new());

    assert_eq!(
        request_expr(&mut cx, data.as_ref(), input.clone()),
        request_expr(&mut cx, &rust, input)
    );
}

#[test]
fn public_star_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "hub"),
        star_hub_connection(),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "spoke-b"),
        fixed_reply_connection("spoke-b"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "spoke-c"),
        fixed_reply_connection("spoke-c"),
    );
    let hub = cx.resolve_value(&Symbol::qualified("test", "hub")).unwrap();
    let spokes = vec![
        cx.resolve_value(&Symbol::qualified("test", "spoke-b"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "spoke-c"))
            .unwrap(),
    ];

    let rust = rust_star(&mut cx);
    let data = build_star_data_graph_connection(
        &mut cx,
        hub,
        spokes,
        Symbol::new("planner"),
        Symbol::new("worker"),
    )
    .unwrap();
    let input = Expr::String("task".to_owned());

    assert_eq!(
        request_expr(&mut cx, data.as_ref(), input.clone()),
        request_expr(&mut cx, &rust, input)
    );
}

#[test]
fn public_mesh_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "mesh-a"),
        fixed_reply_connection("perfect target"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "mesh-b"),
        fixed_reply_connection("target maybe"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "mesh-c"),
        fixed_reply_connection("noise"),
    );
    let judge = rubric_judge(&mut cx, "perfect target");
    let agents = vec![
        cx.resolve_value(&Symbol::qualified("test", "mesh-a"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "mesh-b"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "mesh-c"))
            .unwrap(),
    ];

    let rust = rust_mesh(&mut cx, judge.clone());
    let data = build_mesh_data_graph_connection(&mut cx, agents, judge, 2).unwrap();
    let input = Expr::String("seed".to_owned());
    let data_expr = request_expr(&mut cx, data.as_ref(), input.clone());
    let rust_expr = request_expr(&mut cx, &rust, input);

    assert_eq!(
        map_expr_field(&data_expr, "candidate"),
        map_expr_field(&rust_expr, "candidate")
    );
    assert_eq!(data_expr, rust_expr);
}

#[test]
fn public_market_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    let _cheap = register_bid_worker(
        &mut cx,
        Symbol::qualified("test", "cheap"),
        1.0,
        "cheap won",
    );
    let _expensive = register_bid_worker(
        &mut cx,
        Symbol::qualified("test", "expensive"),
        9.0,
        "expensive won",
    );
    let router = bid_router(&mut cx);
    let workers = vec![
        cx.resolve_value(&Symbol::qualified("test", "expensive"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "cheap"))
            .unwrap(),
    ];

    let rust = rust_market(&mut cx, router.clone());
    let data = build_market_data_graph_connection(&mut cx, workers, router).unwrap();
    let input = Expr::String("route".to_owned());

    assert_eq!(
        request_expr(&mut cx, data.as_ref(), input.clone()),
        request_expr(&mut cx, &rust, input)
    );
}

#[test]
fn public_debate_constructor_matches_normalized_graph_record() {
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
    let pro = cx
        .resolve_value(&Symbol::qualified("test", "debate-pro"))
        .unwrap();
    let con = cx
        .resolve_value(&Symbol::qualified("test", "debate-con"))
        .unwrap();

    let rust = rust_debate(&mut cx, judge.clone());
    let data = build_debate_data_graph_connection(&mut cx, pro, con, judge, 1).unwrap();
    let input = Expr::String("topic".to_owned());

    assert_eq!(
        request_expr(&mut cx, data.as_ref(), input.clone()),
        request_expr(&mut cx, &rust, input)
    );
}

#[test]
fn public_speculate_verify_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "spec-fast"),
        fixed_reply_connection("fast"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "verify-different"),
        verifier_connection("different", "slow"),
    );
    let speculator = cx
        .resolve_value(&Symbol::qualified("test", "spec-fast"))
        .unwrap();
    let verifier = cx
        .resolve_value(&Symbol::qualified("test", "verify-different"))
        .unwrap();

    let rust = rust_speculate_verify(&mut cx);
    let data = build_speculate_verify_data_graph_connection(
        &mut cx,
        speculator,
        verifier,
        Symbol::new("retry"),
    )
    .unwrap();
    let input = Expr::String("task".to_owned());

    assert_eq!(
        request_expr(&mut cx, data.as_ref(), input.clone()),
        request_expr(&mut cx, &rust, input)
    );
}

#[test]
fn public_open_claw_constructor_matches_normalized_graph_record() {
    let mut cx = topology_cx();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "open-a"),
        tagged_append_connection("a"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "open-b"),
        tagged_append_connection("b"),
    );
    let steps = vec![
        cx.resolve_value(&Symbol::qualified("test", "open-a"))
            .unwrap(),
        cx.resolve_value(&Symbol::qualified("test", "open-b"))
            .unwrap(),
    ];

    let rust = rust_open_claw(&mut cx);
    let data = build_open_claw_data_graph_connection(&mut cx, steps).unwrap();
    let input = Expr::List(Vec::new());
    let data_expr = request_expr(&mut cx, data.as_ref(), input.clone());
    let rust_expr = request_expr(&mut cx, &rust, input);

    assert_eq!(data_expr, rust_expr);
    assert!(flatten_text(&data_expr).contains("a:none"));
    assert!(flatten_text(&data_expr).contains("b:none"));
}
