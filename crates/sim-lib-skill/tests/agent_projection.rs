#![cfg(feature = "agent")]

use std::sync::Arc;

use sim_codec_binary::BinaryCodecLib;
use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, ShapeRef, Symbol, Value};
use sim_lib_skill::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, SkillRole, install_skill_lib,
    skill_as_tool_symbol, skill_call_capability, skill_install_capability, skill_install_symbol,
    skill_specific_call_capability, skill_transport_value,
};
use sim_shape::{ListShape, NumberValueShape, shape_value};

#[test]
fn agent_manifest_injects_skill_tool_and_runner_loop_calls_skill_callable_once() {
    let mut cx = agent_skill_cx();
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = install_sum_skill(&mut cx);
    let tool = skill_tool(&mut cx);
    let runner = fake_runner(
        &mut cx,
        "skill-agent-fake",
        vec![
            tool_call_response(vec![tool_call(
                "call-skill",
                Symbol::qualified("skill", "math.add"),
                vec![number(2), number(3)],
            )]),
            final_response("continued after skill tool"),
        ],
    );
    let agent = started_agent(&mut cx, vec![runner], vec![tool]);

    let result = agent_call_expr(&mut cx, &agent, model_request("skill tool", Vec::new()));

    assert!(format!("{result:?}").contains("continued after skill tool"));
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn privacy_allow_tools_denies_skill_tool_before_execution() {
    let mut cx = agent_skill_cx();
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = install_sum_skill(&mut cx);
    let tool = skill_tool(&mut cx);
    let runner = fake_runner(
        &mut cx,
        "skill-agent-privacy",
        vec![tool_call_response(vec![tool_call(
            "call-denied",
            Symbol::qualified("skill", "math.add"),
            vec![number(1), number(1)],
        )])],
    );
    let agent = started_agent(&mut cx, vec![runner], vec![tool]);
    let request = model_request(
        "privacy deny skill tool",
        vec![key_expr("allow-tools", Expr::List(Vec::new()))],
    );

    let result = agent_call_expr(&mut cx, &agent, request);

    assert!(format!("{result:?}").contains("privacy policy denied tool skill/math.add"));
    assert_eq!(fixture.call_count(), 0);
}

fn agent_skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    install_skill_lib(&mut cx).unwrap();
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");
    cx.grant(skill_install_capability());
    cx
}

fn install_sum_skill(cx: &mut Cx) -> Arc<FixtureTransport> {
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let transport = skill_transport_value(cx, fixture.clone()).unwrap();
    let card = sum_card().value(cx).unwrap();
    cx.call_function(&skill_install_symbol(), Args::new(vec![transport, card]))
        .unwrap();
    fixture
}

fn skill_tool(cx: &mut Cx) -> Value {
    let target = cx.factory().string("math.add".to_owned()).unwrap();
    cx.call_function(&skill_as_tool_symbol(), Args::new(vec![target]))
        .unwrap()
}

fn started_agent(cx: &mut Cx, runners: Vec<Value>, tools: Vec<Value>) -> Value {
    let runners = manifest_arg(cx, runners);
    let tools = manifest_arg(cx, tools);
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("skill-agent")).unwrap(),
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                runners,
                cx.factory().symbol(Symbol::new(":tools")).unwrap(),
                tools,
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "start"),
        Args::new(vec![agent.clone()]),
    )
    .unwrap();
    agent
}

fn manifest_arg(cx: &mut Cx, mut values: Vec<Value>) -> Value {
    if values.len() == 1 {
        values.remove(0)
    } else {
        cx.factory().list(values).unwrap()
    }
}

fn agent_call_expr(cx: &mut Cx, agent: &Value, request: Expr) -> Expr {
    let request = cx.factory().expr(request).unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "call"),
        Args::new(vec![agent.clone(), request]),
    )
    .unwrap()
    .object()
    .as_expr(cx)
    .unwrap()
}

fn fake_runner(cx: &mut Cx, name: &str, script: Vec<Expr>) -> Value {
    let script_value = cx.factory().expr(Expr::List(script)).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(format!("{name}/model")).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn model_request(task: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        key_expr("model-request", Expr::Bool(true)),
        key_expr("task", Expr::String(task.to_owned())),
        key_expr("messages", Expr::List(Vec::new())),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn tool_call_response(tool_calls: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        key_expr("model-response", Expr::Bool(true)),
        key_expr("runner", Expr::Symbol(Symbol::new("skill-agent-fake"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr("content", Expr::List(Vec::new())),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("tool-call"))),
        key_expr("tool-calls", Expr::List(tool_calls)),
    ])
}

fn final_response(text: &str) -> Expr {
    Expr::Map(vec![
        key_expr("model-response", Expr::Bool(true)),
        key_expr("runner", Expr::Symbol(Symbol::new("skill-agent-fake"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr(
            "content",
            Expr::List(vec![Expr::Map(vec![
                key_expr("type", Expr::Symbol(Symbol::new("text"))),
                key_expr("text", Expr::String(text.to_owned())),
            ])]),
        ),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("stop"))),
        key_expr("text", Expr::String(text.to_owned())),
    ])
}

fn tool_call(id: &str, name: Symbol, args: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        key_expr("id", Expr::String(id.to_owned())),
        key_expr("name", Expr::Symbol(name)),
        key_expr("arguments", Expr::List(args)),
    ])
}

fn sum_card() -> SkillCard {
    SkillCard::fixture(FixtureSkillSpec {
        id: "math.add".to_owned(),
        symbol: Symbol::qualified("skill", "math.add"),
        title: "Add Numbers".to_owned(),
        description: "Add two numbers with a fixture skill.".to_owned(),
        input_shape: sum_args_shape(),
        output_shape: number_shape("sum-result"),
        transport_id: "math".to_owned(),
        operation: "add".to_owned(),
    })
    .with_role(SkillRole::Tool)
}

fn sum_args_shape() -> ShapeRef {
    shape_value(
        Symbol::qualified("skill", "sum-args"),
        Arc::new(ListShape::new(vec![
            Arc::new(NumberValueShape),
            Arc::new(NumberValueShape),
        ])),
    )
}

fn number_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("skill", name.to_owned()),
        Arc::new(NumberValueShape),
    )
}

fn number(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
