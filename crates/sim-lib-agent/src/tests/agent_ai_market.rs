use super::support::{eval_cx, flatten_text, install_agent_lib, install_test_codec};
use sim_codec_chat::{model_error_expr, validate_chat_transcript};
use sim_kernel::{Consistency, EvalMode, EvalRequest, Expr, Symbol, Value};

fn model_request(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
}

fn text_response(runner: Symbol, model: &str, text: &str) -> Expr {
    sim_codec_chat::model_response_expr(
        runner,
        model,
        vec![Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("type")),
                Expr::Symbol(Symbol::new("text")),
            ),
            (
                Expr::Symbol(Symbol::new("text")),
                Expr::String(text.to_owned()),
            ),
        ])],
        Symbol::new("stop"),
    )
}

fn retryable_error(runner: Symbol, model: &str) -> Expr {
    let Expr::Map(mut entries) = model_error_expr(runner, model, "retry later") else {
        unreachable!("model_error_expr returns a map")
    };
    entries.push((Expr::Symbol(Symbol::new("retryable")), Expr::Bool(true)));
    Expr::Map(entries)
}

fn fake_runner(
    cx: &mut sim_kernel::Cx,
    name: &str,
    cost: f64,
    healthy: bool,
    script: Expr,
) -> Value {
    fake_runner_with_delay(cx, name, cost, healthy, 0, script)
}

fn fake_runner_with_delay(
    cx: &mut sim_kernel::Cx,
    name: &str,
    cost: f64,
    healthy: bool,
    delay_ms: u64,
    script: Expr,
) -> Value {
    let mut args = vec![
        cx.factory().symbol(Symbol::new(":name")).unwrap(),
        cx.factory().symbol(Symbol::new(name)).unwrap(),
        cx.factory().symbol(Symbol::new(":model")).unwrap(),
        cx.factory().string(format!("{name}/model")).unwrap(),
        cx.factory().symbol(Symbol::new(":cost-usd")).unwrap(),
        cx.factory()
            .number_literal(Symbol::qualified("numbers", "f64"), cost.to_string())
            .unwrap(),
        cx.factory().symbol(Symbol::new(":healthy")).unwrap(),
        cx.factory().bool(healthy).unwrap(),
        cx.factory().symbol(Symbol::new(":script")).unwrap(),
        cx.factory().expr(Expr::List(vec![script])).unwrap(),
    ];
    if delay_ms > 0 {
        args.push(cx.factory().symbol(Symbol::new(":delay-ms")).unwrap());
        args.push(
            cx.factory()
                .number_literal(Symbol::qualified("numbers", "f64"), delay_ms.to_string())
                .unwrap(),
        );
    }
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        sim_kernel::Args::new(args),
    )
    .unwrap()
}

fn policy(cx: &mut sim_kernel::Cx, fallback: Option<&str>) -> Value {
    let mut args = vec![
        cx.factory().symbol(Symbol::new(":prefer")).unwrap(),
        cx.factory().symbol(Symbol::new("auction")).unwrap(),
        cx.factory().symbol(Symbol::new(":requires")).unwrap(),
        cx.factory()
            .expr(Expr::List(vec![Expr::Symbol(Symbol::new("text"))]))
            .unwrap(),
    ];
    if let Some(fallback) = fallback {
        args.push(cx.factory().symbol(Symbol::new(":fallback")).unwrap());
        args.push(cx.factory().symbol(Symbol::new(fallback)).unwrap());
    }
    cx.call_function(&Symbol::new("model-policy"), sim_kernel::Args::new(args))
        .unwrap()
}

fn market(cx: &mut sim_kernel::Cx, runners: Vec<Value>, policy: Value) -> Value {
    let runner_list = cx.factory().list(runners).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "market"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new("research-market")).unwrap(),
            cx.factory().symbol(Symbol::new(":runners")).unwrap(),
            runner_list,
            cx.factory().symbol(Symbol::new(":policy")).unwrap(),
            policy,
        ]),
    )
    .unwrap()
}

fn realize(cx: &mut sim_kernel::Cx, fabric: &Value, task: &str) -> Expr {
    realize_expr(cx, fabric, model_request(task))
}

fn realize_expr(cx: &mut sim_kernel::Cx, fabric: &Value, expr: Expr) -> Expr {
    let reply = fabric
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(
            cx,
            EvalRequest {
                expr,
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::default(),
                mode: EvalMode::default(),
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    reply.value.object().as_expr(cx).unwrap()
}

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}

#[test]
fn a5_phase10_market_chooses_cheapest_valid_runner() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let expensive = fake_runner(
        &mut cx,
        "expensive",
        0.20,
        true,
        text_response(Symbol::new("expensive"), "expensive/model", "expensive"),
    );
    let cheap = fake_runner(
        &mut cx,
        "cheap",
        0.01,
        true,
        text_response(Symbol::new("cheap"), "cheap/model", "cheap"),
    );
    let policy = policy(&mut cx, None);
    let market = market(&mut cx, vec![expensive, cheap], policy);

    assert!(market.object().as_eval_fabric().is_some());
    let expr = realize(&mut cx, &market, "pick");
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("cheap"));
    assert!(format!("{expr:?}").contains("market-decision"));
}

#[test]
fn a5_phase10_market_skips_unhealthy_runner() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let unhealthy = fake_runner(
        &mut cx,
        "unhealthy",
        0.0,
        false,
        text_response(Symbol::new("unhealthy"), "unhealthy/model", "bad"),
    );
    let healthy = fake_runner(
        &mut cx,
        "healthy",
        0.10,
        true,
        text_response(Symbol::new("healthy"), "healthy/model", "good"),
    );
    let health = cx
        .call_function(
            &Symbol::qualified("runner", "health"),
            sim_kernel::Args::new(vec![unhealthy.clone()]),
        )
        .unwrap();
    assert!(matches!(
        field(&health.object().as_expr(&mut cx).unwrap(), "healthy"),
        Some(Expr::Bool(false))
    ));
    let policy = policy(&mut cx, None);
    let market = market(&mut cx, vec![unhealthy, healthy], policy);

    let expr = realize(&mut cx, &market, "skip");
    assert!(flatten_text(&expr).contains("good"));
}

#[test]
fn a5_phase10_market_uses_fallback_after_retryable_error() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let primary = fake_runner(
        &mut cx,
        "primary",
        0.0,
        true,
        retryable_error(Symbol::new("primary"), "primary/model"),
    );
    let fallback = fake_runner(
        &mut cx,
        "fallback",
        0.50,
        true,
        text_response(Symbol::new("fallback"), "fallback/model", "fallback ok"),
    );
    let policy = policy(&mut cx, Some("fallback"));
    let market = market(&mut cx, vec![primary, fallback], policy);

    let expr = realize(&mut cx, &market, "fallback");
    assert!(flatten_text(&expr).contains("fallback ok"));
    let decision = field(&expr, "market-decision").unwrap();
    assert!(matches!(
        field(decision, "fallback-used"),
        Some(Expr::Bool(true))
    ));
}

#[test]
fn a5_phase10_runner_cards_are_local_discovery_data() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = fake_runner(
        &mut cx,
        "discoverable",
        0.03,
        true,
        text_response(Symbol::new("discoverable"), "discoverable/model", "ok"),
    );
    let card = cx
        .call_function(
            &Symbol::qualified("runner", "card"),
            sim_kernel::Args::new(vec![runner.clone()]),
        )
        .unwrap();
    let card_expr = card.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&card_expr).unwrap();
    assert!(matches!(
        field(&card_expr, "runner"),
        Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "discoverable"
    ));

    let runner_list = cx.factory().list(vec![runner]).unwrap();
    let cards = cx
        .call_function(
            &Symbol::qualified("runner", "cards"),
            sim_kernel::Args::new(vec![runner_list]),
        )
        .unwrap();
    assert!(matches!(
        cards.object().as_expr(&mut cx).unwrap(),
        Expr::List(items) if items.len() == 1
    ));
}
