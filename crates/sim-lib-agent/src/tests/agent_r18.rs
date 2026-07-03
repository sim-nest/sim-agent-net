use super::agent_r18_support::{
    fixed_reply_connection, map_expr_field, register_bid_worker, register_connection,
    star_hub_connection, tagged_append_connection, verifier_connection,
};
use super::support::{eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs};
use sim_kernel::{Args, Expr, Symbol};
use sim_lib_server::Connection;

#[test]
fn r18_ring_cycles_roles_and_visits_agents_in_order() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
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

    let ring = cx
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

    let reply = ring
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::List(Vec::new()), None, Vec::new())
        .unwrap();
    let expr = reply.object().as_expr(&mut cx).unwrap();
    let text = flatten_text(&expr);
    assert!(text.contains("a:planner"));
    assert!(text.contains("b:worker"));
    assert!(text.contains("c:critic"));
}

#[test]
fn r18_star_fans_out_and_hub_sees_all_spoke_replies() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
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

    let topology = cx
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
    let expr = topology
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("task".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let text = flatten_text(&expr);
    assert!(text.contains("spoke-b"));
    assert!(text.contains("spoke-c"));
    assert!(text.contains("merge-count"));
}

#[test]
fn r18_mesh_uses_real_judge_to_pick_best_answer() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
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

    let judge = cx
        .call_function(
            &Symbol::qualified("judge", "rubric"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory().string("perfect target".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let topology = cx
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
    let expr = topology
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("seed".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&expr, "candidate"),
        Expr::String("perfect target".to_owned())
    );
}

#[test]
fn r18_market_routes_to_lowest_bidder() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

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
    let router = cx
        .call_function(
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
        .unwrap();
    let topology = cx
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
    let expr = topology
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("route".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(flatten_text(&expr).contains("cheap"));
    assert!(flatten_text(&expr).contains("cheap won"));
}

#[test]
fn r18_debate_returns_verdict_winner_and_transcript() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let judge = cx
        .call_function(
            &Symbol::qualified("judge", "rubric"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory().string("pro wins evidence".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
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
    let topology = cx
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
    let expr = topology
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("topic".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&expr, "winner"),
        Expr::String("pro".to_owned())
    );
    let text = flatten_text(&expr);
    assert!(text.contains("pro wins evidence"));
    assert!(text.contains("con loses"));
}

#[test]
fn r18_speculate_verify_uses_fast_path_and_handles_mismatch() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "spec-fast"),
        fixed_reply_connection("fast"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "verify-fast"),
        verifier_connection("fast", "slow"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "verify-different"),
        verifier_connection("different", "slow"),
    );

    let agree = cx
        .call_function(
            &Symbol::qualified("topology", "speculate-verify"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":speculator")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "spec-fast"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":verifier")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "verify-fast"))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let agree_expr = agree
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("task".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&agree_expr, "result"),
        Expr::String("fast".to_owned())
    );
    assert_eq!(map_expr_field(&agree_expr, "agreed"), Expr::Bool(true));

    let mismatch = cx
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
    let mismatch_expr = mismatch
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(&mut cx, Expr::String("task".to_owned()), None, Vec::new())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(map_expr_field(&mismatch_expr, "agreed"), Expr::Bool(false));
    assert_eq!(
        map_expr_field(&mismatch_expr, "result"),
        Expr::String("slow".to_owned())
    );
}
