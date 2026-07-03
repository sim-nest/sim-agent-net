use super::support::{eval_cx, flatten_text, install_agent_lib, install_test_codec};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Consistency, EvalMode, EvalRequest, Expr, Symbol, Value};

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

fn fake_runner(cx: &mut sim_kernel::Cx, name: &str, cost: f64, script: Expr) -> Value {
    fake_runner_with_delay(cx, name, cost, 0, script)
}

fn fake_runner_with_delay(
    cx: &mut sim_kernel::Cx,
    name: &str,
    cost: f64,
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
        cx.factory().bool(true).unwrap(),
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

fn policy(
    cx: &mut sim_kernel::Cx,
    execution: &str,
    fallback: Option<&str>,
    verify_with: Option<&str>,
    judge: Option<&str>,
) -> Value {
    let mut args = vec![
        cx.factory().symbol(Symbol::new(":prefer")).unwrap(),
        cx.factory().symbol(Symbol::new("auction")).unwrap(),
        cx.factory().symbol(Symbol::new(":execution")).unwrap(),
        cx.factory().symbol(Symbol::new(execution)).unwrap(),
        cx.factory().symbol(Symbol::new(":requires")).unwrap(),
        cx.factory()
            .expr(Expr::List(vec![Expr::Symbol(Symbol::new("text"))]))
            .unwrap(),
    ];
    if let Some(fallback) = fallback {
        args.push(cx.factory().symbol(Symbol::new(":fallback")).unwrap());
        args.push(cx.factory().symbol(Symbol::new(fallback)).unwrap());
    }
    if let Some(verify_with) = verify_with {
        args.push(cx.factory().symbol(Symbol::new(":verify-with")).unwrap());
        args.push(cx.factory().symbol(Symbol::new(verify_with)).unwrap());
    }
    if let Some(judge) = judge {
        args.push(cx.factory().symbol(Symbol::new(":judge")).unwrap());
        args.push(cx.factory().symbol(Symbol::new(judge)).unwrap());
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
            cx.factory().symbol(Symbol::new("phase7-market")).unwrap(),
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
fn a6_phase7_market_race_chooses_earliest_successful_runner() {
    let mut cx = phase7_cx();
    let slow = fake_runner_with_delay(
        &mut cx,
        "slow",
        0.01,
        50,
        text_response(Symbol::new("slow"), "slow/model", "slow"),
    );
    let fast = fake_runner_with_delay(
        &mut cx,
        "fast",
        0.20,
        1,
        text_response(Symbol::new("fast"), "fast/model", "fast"),
    );
    let policy = policy(&mut cx, "race", None, None, None);
    let market = market(&mut cx, vec![slow, fast], policy);

    let expr = realize(&mut cx, &market, "race");
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("fast"));
    assert!(format!("{:?}", field(&expr, "market-decision")).contains("cancellations"));
}

#[test]
fn a6_phase7_market_speculate_keeps_expensive_when_verifier_rejects_cheap() {
    let mut cx = phase7_cx();
    let cheap = fake_runner(
        &mut cx,
        "cheap",
        0.01,
        text_response(Symbol::new("cheap"), "cheap/model", "cheap"),
    );
    let expensive = fake_runner(
        &mut cx,
        "expensive",
        0.50,
        text_response(Symbol::new("expensive"), "expensive/model", "expensive"),
    );
    let verifier = fake_runner(
        &mut cx,
        "verifier",
        0.0,
        text_response(Symbol::new("verifier"), "verifier/model", "reject"),
    );
    let policy = policy(&mut cx, "speculate", None, Some("verifier"), None);
    let market = market(&mut cx, vec![cheap, expensive, verifier], policy);

    let expr = realize(&mut cx, &market, "speculate");
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("expensive"));
    let decision = field(&expr, "market-decision").unwrap();
    assert!(matches!(
        field(decision, "verified-by"),
        Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "verifier"
    ));
}

#[test]
fn a6_phase7_market_debate_uses_judge_over_candidate_answers() {
    let mut cx = phase7_cx();
    let pro = fake_runner(
        &mut cx,
        "pro",
        0.10,
        text_response(Symbol::new("pro"), "pro/model", "pro answer"),
    );
    let con = fake_runner(
        &mut cx,
        "con",
        0.10,
        text_response(Symbol::new("con"), "con/model", "con answer"),
    );
    let judge = fake_runner(
        &mut cx,
        "judge",
        0.20,
        text_response(Symbol::new("judge"), "judge/model", "judge verdict"),
    );
    let policy = policy(&mut cx, "debate", None, None, Some("judge"));
    let market = market(&mut cx, vec![pro, con, judge], policy);

    let expr = realize(&mut cx, &market, "debate");
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("judge verdict"));
    assert!(format!("{expr:?}").contains("debate-answers"));
}

#[test]
fn a6_phase7_market_escalates_after_shape_failure() {
    let mut cx = phase7_cx();
    let bad = fake_runner(
        &mut cx,
        "bad-shape",
        0.01,
        Expr::String("not a package".to_owned()),
    );
    let fallback = fake_runner(
        &mut cx,
        "fallback-shape",
        0.20,
        value_response(
            Symbol::new("fallback-shape"),
            "fallback-shape/model",
            valid_package_expr(),
        ),
    );
    let policy = policy(&mut cx, "escalate", Some("fallback-shape"), None, None);
    let market = market(&mut cx, vec![bad, fallback], policy);

    let expr = realize_expr(&mut cx, &market, shape_request(output_package_shape()));
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("sim-say"));
    let decision = field(&expr, "market-decision").unwrap();
    assert!(matches!(
        field(decision, "fallback-used"),
        Some(Expr::Bool(true))
    ));
}

fn phase7_cx() -> sim_kernel::Cx {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn shape_request(output_shape: Expr) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("shape-check".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (Expr::Symbol(Symbol::new("output-shape")), output_shape),
    ])
}

fn output_package_shape() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::new("fields")),
        Expr::List(vec![
            Expr::Symbol(Symbol::new(":name")),
            Expr::Symbol(Symbol::new("String")),
        ]),
        Expr::List(vec![
            Expr::Symbol(Symbol::new(":version")),
            Expr::Symbol(Symbol::new("String")),
        ]),
    ])
}

fn valid_package_expr() -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("name")),
            Expr::String("sim-say".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("version")),
            Expr::String("0.1.0".to_owned()),
        ),
    ])
}

fn value_response(runner: Symbol, model: &str, value: Expr) -> Expr {
    sim_codec_chat::model_response_expr(
        runner,
        model,
        vec![Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("type")),
                Expr::Symbol(Symbol::new("value")),
            ),
            (Expr::Symbol(Symbol::new("value")), value),
        ])],
        Symbol::new("stop"),
    )
}
