use super::agent_r18_support::{
    fixed_reply_connection, map_expr_field, register_bid_worker, register_connection,
    star_hub_connection, tagged_append_connection, verifier_connection,
};
use super::support::{eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs};
use crate::agents::topology_data::{
    build_debate_data_graph_connection, build_market_data_graph_connection,
    build_mesh_data_graph_connection, build_open_claw_data_graph_connection,
    build_ring_data_graph_connection, build_speculate_verify_data_graph_connection,
    build_star_data_graph_connection,
};
use sim_kernel::{Args, CapabilitySet, Consistency, Cx, EvalMode, EvalRequest, Expr, Symbol};
use sim_lib_agent_conduct_core::{AgentRunFrame, AgentUsageBudget, UsageQuantity};
use sim_lib_server::Connection;
use sim_lib_topology::topology_run_capability;

use crate::execute_delegate_once;

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
        Symbol::qualified("child", "debate"),
        allowed.clone(),
        charge.clone(),
        &budget,
        request(Expr::String("topic".into())),
    )
    .unwrap();
    let market_result = execute_delegate_once(
        &mut cx,
        &mut parent,
        market.as_ref(),
        Symbol::qualified("child", "market"),
        allowed,
        charge,
        &budget,
        request(Expr::String("route".into())),
    )
    .unwrap();

    assert!(map_expr_field(&debate_result.output, "winner").is_some());
    assert!(map_expr_field(&debate_result.output, "verdict").is_some());
    assert!(map_expr_field(&market_result.output, "winner").is_some());
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
