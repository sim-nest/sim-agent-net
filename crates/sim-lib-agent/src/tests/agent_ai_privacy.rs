use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_test_codec, register_sum_tool,
    request_frame,
};
use sim_kernel::{Args, Cx, Expr, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerFrame, eval_reply_from_frame};

#[test]
fn a6_phase6_local_only_permits_local_fake_runner() {
    let mut cx = privacy_cx();
    let runner = fake_runner(&mut cx, "local-fake", vec![final_response("local ok")]);
    let (expr, _) = runner_answer_expr(
        &mut cx,
        &runner,
        model_request("local prompt", vec![privacy_symbol("local-only")]),
    );

    assert!(flatten_text(&expr).contains("local ok"));
}

#[test]
fn a6_phase6_metadata_only_recorder_hashes_payload_text() {
    let mut cx = privacy_cx();
    cx.grant_named("agent-spawn");
    let recorder = recorder_journal(&mut cx);
    let runner = fake_runner(
        &mut cx,
        "private-fake",
        vec![final_response("secret response text")],
    );
    let agent = started_agent(&mut cx, vec![runner], Vec::new(), vec![recorder.clone()]);

    let (expr, _) = agent_answer_expr(
        &mut cx,
        &agent,
        model_request("secret prompt text", vec![privacy_symbol("metadata-only")]),
    );

    assert!(flatten_text(&expr).contains("secret response text"));
    let snapshot = recorder_snapshot(&mut cx, &recorder);
    let snapshot_text = format!("{snapshot:?}");
    assert!(snapshot_text.contains("privacy-redacted"));
    assert!(snapshot_text.contains("payload-hash"));
    assert!(!snapshot_text.contains("secret prompt text"));
    assert!(!snapshot_text.contains("secret response text"));
}

#[test]
fn a6_phase6_allow_tools_denies_prompt_content_to_unlisted_tool() {
    let mut cx = privacy_cx();
    cx.grant_named("agent-spawn");
    cx.grant_named("math");
    let tool = register_sum_tool(&mut cx);
    let tool_value = cx.resolve_value(&tool.symbol).unwrap();
    let runner = fake_runner(
        &mut cx,
        "tool-private-fake",
        vec![tool_call_response(vec![tool_call(
            "call-1",
            Symbol::qualified("test", "sum"),
            vec![number(2), number(3)],
        )])],
    );
    let agent = started_agent(&mut cx, vec![runner], vec![tool_value], Vec::new());

    let (expr, _) = agent_answer_expr(
        &mut cx,
        &agent,
        model_request(
            "private tool prompt",
            vec![key_expr(
                "privacy",
                Expr::Map(vec![key_expr("allow-tools", Expr::List(Vec::new()))]),
            )],
        ),
    );

    assert!(flatten_text(&expr).contains("privacy policy denied tool test/sum"));
}

fn privacy_cx() -> Cx {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn runner_answer_expr(
    cx: &mut Cx,
    runner: &Value,
    request: Expr,
) -> (Expr, Vec<sim_kernel::Diagnostic>) {
    let frame = request_frame(cx, request);
    let reply = as_component(runner).answer(cx, frame).unwrap();
    let reply = eval_reply_from_frame(cx, &reply).unwrap();
    (reply.value.object().as_expr(cx).unwrap(), reply.diagnostics)
}

fn started_agent(
    cx: &mut Cx,
    runners: Vec<Value>,
    tools: Vec<Value>,
    recorders: Vec<Value>,
) -> Value {
    let runners = manifest_arg(cx, runners);
    let tools = manifest_arg(cx, tools);
    let recorders = manifest_arg(cx, recorders);
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("phase6-agent")).unwrap(),
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                runners,
                cx.factory().symbol(Symbol::new(":tools")).unwrap(),
                tools,
                cx.factory().symbol(Symbol::new(":recorders")).unwrap(),
                recorders,
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

fn agent_answer_expr(
    cx: &mut Cx,
    agent: &Value,
    request: Expr,
) -> (Expr, Vec<sim_kernel::Diagnostic>) {
    let frame = agent_answer_frame(cx, agent, request).unwrap();
    let reply = eval_reply_from_frame(cx, &frame).unwrap();
    let expr = reply.value.object().as_expr(cx).unwrap();
    (expr, reply.diagnostics)
}

fn agent_answer_frame(cx: &mut Cx, agent: &Value, request: Expr) -> Result<ServerFrame> {
    let agent = agent.object().downcast_ref::<crate::Agent>().unwrap();
    let frame = request_frame(cx, request);
    agent.site()?.answer(cx, frame)
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

fn recorder_journal(cx: &mut Cx) -> Value {
    cx.call_function(
        &Symbol::qualified("recorder", "journal"),
        Args::new(Vec::new()),
    )
    .unwrap()
}

fn recorder_snapshot(cx: &mut Cx, recorder: &Value) -> Expr {
    let frame = request_frame(cx, Expr::List(vec![Expr::Symbol(Symbol::new("snapshot"))]));
    let reply = as_component(recorder).answer(cx, frame).unwrap();
    eval_reply_from_frame(cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(cx)
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

fn final_response(text: &str) -> Expr {
    Expr::Map(vec![
        key_expr("model-response", Expr::Bool(true)),
        key_expr("runner", Expr::Symbol(Symbol::new("phase6-fake"))),
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

fn tool_call_response(tool_calls: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        key_expr("model-response", Expr::Bool(true)),
        key_expr("runner", Expr::Symbol(Symbol::new("phase6-fake"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr("content", Expr::List(Vec::new())),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("tool-call"))),
        key_expr("tool-calls", Expr::List(tool_calls)),
    ])
}

fn tool_call(id: &str, name: Symbol, args: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        key_expr("id", Expr::String(id.to_owned())),
        key_expr("name", Expr::Symbol(name)),
        key_expr("arguments", Expr::List(args)),
    ])
}

fn number(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn privacy_symbol(policy: &str) -> (Expr, Expr) {
    key_expr("privacy", Expr::Symbol(Symbol::new(policy)))
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
