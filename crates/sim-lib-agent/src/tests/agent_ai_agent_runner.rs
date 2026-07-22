use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_test_codec, request_frame,
};
use crate::{
    AgentComponent, ComponentBackend, ComponentKind, RunnerBackend, components::component_value,
    util::installed_codecs,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{
    CapabilityName, Consistency, Error, EvalMode, EvalRequest, Expr, Result, Symbol, Value,
};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_lib_server::{EvalSite, ServerAddress, eval_reply_from_frame};
use std::sync::Arc;

const NESTED_CAPABILITY: &str = "agent-runner-nested-ceiling";

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

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}

fn realize_with_capabilities(
    cx: &mut sim_kernel::Cx,
    fabric: &Value,
    task: &str,
    required_capabilities: Vec<CapabilityName>,
) -> Result<Expr> {
    let reply = fabric.object().as_eval_fabric().unwrap().realize(
        cx,
        EvalRequest {
            expr: model_request(task),
            result_shape: None,
            required_capabilities,
            deadline: None,
            consistency: Consistency::default(),
            mode: EvalMode::default(),
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        },
    )?;
    reply.value.object().as_expr(cx)
}

fn realize(cx: &mut sim_kernel::Cx, fabric: &Value, task: &str) -> Expr {
    realize_with_capabilities(cx, fabric, task, Vec::new()).unwrap()
}

fn fake_runner(cx: &mut sim_kernel::Cx, name: &str, cost: f64, script: Expr) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(format!("{name}/model")).unwrap(),
            cx.factory().symbol(Symbol::new(":cost-usd")).unwrap(),
            cx.factory()
                .number_literal(Symbol::qualified("numbers", "f64"), cost.to_string())
                .unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            cx.factory().expr(Expr::List(vec![script])).unwrap(),
        ]),
    )
    .unwrap()
}

fn policy(cx: &mut sim_kernel::Cx) -> Value {
    cx.call_function(
        &Symbol::new("model-policy"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":prefer")).unwrap(),
            cx.factory().symbol(Symbol::new("auction")).unwrap(),
            cx.factory().symbol(Symbol::new(":requires")).unwrap(),
            cx.factory()
                .expr(Expr::List(vec![Expr::Symbol(Symbol::new("text"))]))
                .unwrap(),
        ]),
    )
    .unwrap()
}

fn market(cx: &mut sim_kernel::Cx, runners: Vec<Value>) -> Value {
    let runner_list = cx.factory().list(runners).unwrap();
    let policy = policy(cx);
    cx.call_function(
        &Symbol::qualified("runner", "market"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new("agent-market")).unwrap(),
            cx.factory().symbol(Symbol::new(":runners")).unwrap(),
            runner_list,
            cx.factory().symbol(Symbol::new(":policy")).unwrap(),
            policy,
        ]),
    )
    .unwrap()
}

#[derive(Clone)]
struct RequiredCapabilityRunner {
    symbol: Symbol,
    model: String,
    required: CapabilityName,
}

impl ModelRunner for RequiredCapabilityRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            self.symbol.clone(),
            self.model.clone(),
            Symbol::new("fixture"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, cx: &mut sim_kernel::Cx, request: ModelRequest) -> Result<ModelResponse> {
        self.response(cx, request)
    }

    fn infer_request(
        &self,
        cx: &mut sim_kernel::Cx,
        request: EvalRequest,
    ) -> Result<ModelResponse> {
        if request
            .required_capabilities
            .iter()
            .any(|capability| capability == &self.required)
        {
            cx.require(&self.required)?;
        }
        self.response(cx, ModelRequest::try_from(request.expr)?)
    }
}

impl RequiredCapabilityRunner {
    fn response(&self, _cx: &mut sim_kernel::Cx, request: ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse::new(
            self.symbol.clone(),
            self.model.clone(),
            vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("type")),
                    Expr::Symbol(Symbol::new("text")),
                ),
                (
                    Expr::Symbol(Symbol::new("text")),
                    Expr::String(format!("capability propagated {:?}", request.task)),
                ),
            ])],
            Symbol::new("stop"),
        ))
    }
}

fn required_capability_runner(cx: &mut sim_kernel::Cx, name: &str) -> Value {
    let symbol = Symbol::new(name);
    component_value(
        cx,
        AgentComponent {
            symbol: symbol.clone(),
            kind: ComponentKind::Runner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: Vec::new(),
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: Arc::new(RequiredCapabilityRunner {
                    symbol,
                    model: format!("{name}/model"),
                    required: CapabilityName::new(NESTED_CAPABILITY),
                }),
            }),
        },
    )
    .unwrap()
}

fn assert_nested_denied(error: Error) {
    assert!(matches!(
        error,
        Error::CapabilityDenied { capability }
            if capability == CapabilityName::new(NESTED_CAPABILITY)
    ));
}

fn nested_capabilities() -> Vec<CapabilityName> {
    vec![CapabilityName::new(NESTED_CAPABILITY)]
}

fn started_identity_agent(cx: &mut sim_kernel::Cx) -> Value {
    cx.grant_named("agent-spawn");
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("identity-agent")).unwrap(),
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "start"),
        sim_kernel::Args::new(vec![agent.clone()]),
    )
    .unwrap();
    agent
}

fn agent_runner(cx: &mut sim_kernel::Cx, agent: Value, cost: f64) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "agent"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new("agent-model")).unwrap(),
            cx.factory().symbol(Symbol::new(":agent")).unwrap(),
            agent,
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory()
                .string("sim-agent/identity-agent".to_owned())
                .unwrap(),
            cx.factory().symbol(Symbol::new(":cost-usd")).unwrap(),
            cx.factory()
                .number_literal(Symbol::qualified("numbers", "f64"), cost.to_string())
                .unwrap(),
        ]),
    )
    .unwrap()
}

#[test]
fn a5_phase11_runner_agent_wraps_started_agent() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let agent = started_identity_agent(&mut cx);
    let runner = agent_runner(&mut cx, agent, 0.01);
    let card = cx
        .call_function(
            &Symbol::qualified("runner", "card"),
            sim_kernel::Args::new(vec![runner.clone()]),
        )
        .unwrap();
    let card_expr = card.object().as_expr(&mut cx).unwrap();
    assert!(matches!(
        field(&card_expr, "locality"),
        Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "agent"
    ));

    let expr = realize(&mut cx, &runner, "phase 11");
    validate_chat_transcript(&expr).unwrap();
    assert!(matches!(
        field(&expr, "runner"),
        Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "agent-model"
    ));
    assert!(field(&expr, "agent-task-id").is_some());
    assert!(flatten_text(&expr).contains("phase 11"));
}

#[test]
fn a5_phase11_market_routes_to_agent_backed_runner() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let agent = started_identity_agent(&mut cx);
    let agent_runner = agent_runner(&mut cx, agent, 0.01);
    let fake = fake_runner(
        &mut cx,
        "slower-fake",
        0.05,
        text_response(Symbol::new("slower-fake"), "slower-fake/model", "fake"),
    );
    let market = market(&mut cx, vec![fake, agent_runner]);
    let expr = realize(&mut cx, &market, "route to agent");

    validate_chat_transcript(&expr).unwrap();
    assert!(field(&expr, "agent-task-id").is_some());
    assert!(format!("{expr:?}").contains("market-decision"));
}

#[test]
fn a5_phase11_agent_runner_trace_uses_inner_agent_task_id() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");

    let recorder = cx
        .call_function(
            &Symbol::qualified("recorder", "journal"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("traced-agent")).unwrap(),
                cx.factory().symbol(Symbol::new(":recorders")).unwrap(),
                recorder.clone(),
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "start"),
        sim_kernel::Args::new(vec![agent.clone()]),
    )
    .unwrap();

    let runner = agent_runner(&mut cx, agent, 0.01);
    let expr = realize(&mut cx, &runner, "trace via agent");
    let Some(Expr::String(task_id)) = field(&expr, "agent-task-id") else {
        panic!("runner/agent response should include agent-task-id");
    };

    let snapshot = request_frame(
        &mut cx,
        Expr::List(vec![Expr::Symbol(Symbol::new("snapshot"))]),
    );
    let reply = as_component(&recorder).answer(&mut cx, snapshot).unwrap();
    let trace_expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let trace_text = format!("{trace_expr:?}");
    assert!(trace_text.contains(task_id));
    assert!(trace_text.contains("trace via agent"));
}

#[test]
fn a5_phase11_three_runner_debate_produces_judged_answer() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let first = fake_runner(
        &mut cx,
        "first",
        0.01,
        text_response(Symbol::new("first"), "first/model", "answer one"),
    );
    let second = fake_runner(
        &mut cx,
        "second",
        0.01,
        text_response(Symbol::new("second"), "second/model", "answer two"),
    );
    let third = fake_runner(
        &mut cx,
        "third",
        0.01,
        text_response(Symbol::new("third"), "third/model", "answer three"),
    );
    let judge = fake_runner(
        &mut cx,
        "judge",
        0.01,
        text_response(Symbol::new("judge"), "judge/model", "judged answer"),
    );
    let runners = cx.factory().list(vec![first, second, third]).unwrap();
    let debate = cx
        .call_function(
            &Symbol::qualified("runner", "debate"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("debate-model")).unwrap(),
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                runners,
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                judge,
            ]),
        )
        .unwrap();

    let expr = realize(&mut cx, &debate, "choose");
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("judged answer"));
    assert!(matches!(
        field(&expr, "debate-answers"),
        Some(Expr::List(items)) if items.len() == 3
    ));
}

#[test]
fn runner_agent_preserves_required_capability_ceiling() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let target = required_capability_runner(&mut cx, "ceiling-agent-target");
    let runner = agent_runner(&mut cx, target, 0.01);

    let denied = realize_with_capabilities(
        &mut cx,
        &runner,
        "agent ceiling denied",
        nested_capabilities(),
    )
    .unwrap_err();
    assert_nested_denied(denied);

    cx.grant_named(NESTED_CAPABILITY);
    let allowed = realize_with_capabilities(
        &mut cx,
        &runner,
        "agent ceiling allowed",
        nested_capabilities(),
    )
    .unwrap();
    assert!(flatten_text(&allowed).contains("capability propagated"));
}

#[test]
fn runner_debate_preserves_required_capability_ceiling() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let candidate = required_capability_runner(&mut cx, "ceiling-debate-candidate");
    let judge = required_capability_runner(&mut cx, "ceiling-debate-judge");
    let debate = cx
        .call_function(
            &Symbol::qualified("runner", "debate"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("ceiling-debate")).unwrap(),
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                cx.factory().list(vec![candidate]).unwrap(),
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                judge,
            ]),
        )
        .unwrap();

    let denied = realize_with_capabilities(
        &mut cx,
        &debate,
        "debate ceiling denied",
        nested_capabilities(),
    )
    .unwrap_err();
    assert_nested_denied(denied);

    cx.grant_named(NESTED_CAPABILITY);
    let allowed = realize_with_capabilities(
        &mut cx,
        &debate,
        "debate ceiling allowed",
        nested_capabilities(),
    )
    .unwrap();
    assert!(flatten_text(&allowed).contains("capability propagated"));
}

#[test]
fn runner_market_preserves_required_capability_ceiling() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = required_capability_runner(&mut cx, "ceiling-market-candidate");
    let market = market(&mut cx, vec![runner]);

    let denied = realize_with_capabilities(
        &mut cx,
        &market,
        "market ceiling denied",
        nested_capabilities(),
    )
    .unwrap_err();
    assert_nested_denied(denied);

    cx.grant_named(NESTED_CAPABILITY);
    let allowed = realize_with_capabilities(
        &mut cx,
        &market,
        "market ceiling allowed",
        nested_capabilities(),
    )
    .unwrap();
    assert!(flatten_text(&allowed).contains("capability propagated"));
}
