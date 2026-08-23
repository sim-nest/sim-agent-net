use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use sim_codec_binary::BinaryCodecLib;
use sim_kernel::{
    Args, Consistency, Cx, DefaultFactory, EvalMode, EvalRequest, Expr, HandleSeed, Result,
    ShapeRef, Symbol, Value,
};
use sim_lib_agent::{
    ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
};
use sim_shape::{AnyShape, ListShape, shape_value};

use super::{skill_as_runner_symbol, skill_model_runner};
use crate::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, SkillEventSink, SkillRole,
    SkillTransport, install_skill_lib, skill_call_capability, skill_install_capability,
    skill_install_symbol, skill_specific_call_capability, skill_transport_value,
};

#[test]
fn skill_as_runner_returns_existing_runner_card_projection() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let (_fixture, _card) = install_model_skill(&mut cx, "foo.answer", "answer");
    let runner = as_runner(&mut cx, "foo.answer");

    let card = cx
        .call_function(
            &Symbol::qualified("runner", "card"),
            Args::new(vec![runner]),
        )
        .unwrap();
    let card = ModelCard::try_from(card.object().as_expr(&mut cx).unwrap()).unwrap();

    assert_eq!(card.runner, Symbol::qualified("skill", "foo.answer"));
    assert_eq!(card.model, "skill/foo.answer");
    assert_eq!(card.provider, Symbol::new("skill"));
    assert!(format!("{:?}", card.extra).contains("supports-stream"));
}

#[test]
fn runner_market_can_bid_on_and_execute_skill_model() {
    let mut cx = skill_cx();
    grant_model_caps(&mut cx, "foo.answer");
    let (fixture, _card) = install_model_skill(&mut cx, "foo.answer", "answer");
    let runner = as_runner(&mut cx, "foo.answer");
    let runners = cx.factory().list(vec![runner]).unwrap();
    let runners_key = keyword(&mut cx, ":runners");
    let market = cx
        .call_function(
            &Symbol::qualified("runner", "market"),
            Args::new(vec![runners_key, runners]),
        )
        .unwrap();

    let response = realize_runner(&mut cx, &market, model_request("market request"));

    assert!(format!("{:?}", response.content).contains("skill model answer"));
    assert_eq!(response.runner, Symbol::qualified("skill", "foo.answer"));
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn skill_runner_forwards_stream_events_to_transport_sink() {
    let mut cx = skill_cx();
    grant_model_caps(&mut cx, "stream.answer");
    let transport = Arc::new(EventTransport::new("stream"));
    let card = model_card("stream.answer", "stream", "answer");
    install_with_card(&mut cx, transport.clone(), card.clone());
    let runner = skill_model_runner(&mut cx, &card).unwrap();
    let mut sink = CollectEvents::default();

    let response = runner
        .infer_stream(&mut cx, model_request("stream request"), &mut sink)
        .unwrap();

    assert!(transport.saw_events());
    assert!(format!("{:?}", response.content).contains("streamed answer"));
    assert!(
        sink.events
            .iter()
            .any(|event| event.event == Symbol::new("delta"))
    );
    assert!(
        sink.events
            .iter()
            .any(|event| event.event == Symbol::new("final"))
    );
}

#[test]
fn non_model_skill_is_rejected_as_runner() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let (_fixture, card) = install_non_model_skill(&mut cx, "tool.answer", "answer");

    let err = match skill_model_runner(&mut cx, &card) {
        Ok(_) => panic!("non-model skill should not become a runner"),
        Err(err) => err,
    };

    assert!(format!("{err}").contains("model role"));
}

#[cfg(feature = "openai")]
#[test]
fn openai_skill_model_id_executes_only_registered_model_role_skill() {
    use serde_json::{Value as JsonValue, json};
    use sim_lib_openai_server::{
        DeterministicWallClock, GatewayRequest, MemoryGatewayStore, OpenAiRunnerRegistry,
        RESPONSES_PATH, ResponseIdGenerators, execute_response_request_with_runners,
        install_openai_gateway_lib,
    };

    let mut cx = skill_cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    grant_model_caps(&mut cx, "foo.answer");
    let (fixture, card) = install_model_skill(&mut cx, "foo.answer", "answer");
    let mut runners = OpenAiRunnerRegistry::new();
    runners.register(
        card.symbol.as_qualified_str(),
        Arc::new(skill_model_runner(&mut cx, &card).unwrap()),
    );

    let execution = execute_openai_model(&mut cx, &runners, "skill/foo.answer");
    let body: JsonValue = serde_json::from_slice(execution.response().body()).unwrap();

    assert_eq!(execution.response().status(), 200);
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("skill model answer")
    );
    assert_eq!(fixture.call_count(), 1);

    let empty_runners = OpenAiRunnerRegistry::new();
    let rejected = execute_openai_model(&mut cx, &empty_runners, "skill/tool.answer");
    let body: JsonValue = serde_json::from_slice(rejected.response().body()).unwrap();

    assert_ne!(rejected.response().status(), 200);
    assert!(body.to_string().contains("model_not_found"));

    fn execute_openai_model(
        cx: &mut Cx,
        runners: &OpenAiRunnerRegistry,
        model: &str,
    ) -> sim_lib_openai_server::ResponseExecution {
        let mut store = MemoryGatewayStore::new();
        let mut ids = ResponseIdGenerators::deterministic(1);
        let mut clock = DeterministicWallClock::new(1_000, 10);
        let request = GatewayRequest::new(
            "POST",
            RESPONSES_PATH,
            vec![("Content-Type".to_owned(), "application/json".to_owned())],
            serde_json::to_vec(&json!({
                "model": model,
                "input": "hello from OpenAI",
                "store": false
            }))
            .unwrap(),
        );
        execute_response_request_with_runners(
            cx, &mut store, &mut ids, &mut clock, &request, runners,
        )
    }
}

#[derive(Default)]
struct CollectEvents {
    events: Vec<ModelEvent>,
}

impl ModelEventSink for CollectEvents {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        self.events.push(event);
        Ok(())
    }
}

struct EventTransport {
    id: String,
    saw_events: AtomicBool,
}

impl EventTransport {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            saw_events: AtomicBool::new(false),
        }
    }

    fn saw_events(&self) -> bool {
        self.saw_events.load(Ordering::SeqCst)
    }
}

impl SkillTransport for EventTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "event-test"
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        Ok(Vec::new())
    }

    fn call(
        &self,
        cx: &mut Cx,
        _card: &SkillCard,
        _args: Value,
        events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        if let Some(events) = events {
            self.saw_events.store(true, Ordering::SeqCst);
            let event = ModelEvent::delta_text(
                Symbol::qualified("skill", "stream.answer"),
                "skill/stream.answer",
                Expr::String("span-1".to_owned()),
                "streamed ",
            );
            let event = cx.factory().expr(Expr::from(event))?;
            events.emit(cx, event)?;
        }
        cx.factory().string("streamed answer".to_owned())
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().nil()
    }
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(sim_kernel::EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x5A11_0006),
    );
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    install_skill_lib(&mut cx).unwrap();
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();
    cx
}

fn grant_model_caps(cx: &mut Cx, id: &str) {
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability(id));
}

fn install_model_skill(
    cx: &mut Cx,
    id: &str,
    operation: &str,
) -> (Arc<FixtureTransport>, SkillCard) {
    let fixture = Arc::new(FixtureTransport::new("model"));
    fixture
        .insert(
            operation.to_owned(),
            FixtureBehavior::ConstantString("skill model answer".to_owned()),
        )
        .unwrap();
    let card = model_card(id, "model", operation);
    install_with_card(cx, fixture.clone(), card.clone());
    (fixture, card)
}

fn install_non_model_skill(
    cx: &mut Cx,
    id: &str,
    operation: &str,
) -> (Arc<FixtureTransport>, SkillCard) {
    let fixture = Arc::new(FixtureTransport::new("tool"));
    fixture
        .insert(
            operation.to_owned(),
            FixtureBehavior::ConstantString("tool".to_owned()),
        )
        .unwrap();
    let card = SkillCard::fixture(FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Tool Skill".to_owned(),
        description: "A non-model skill.".to_owned(),
        input_shape: model_args_shape(),
        output_shape: any_shape("tool-result"),
        transport_id: "tool".to_owned(),
        operation: operation.to_owned(),
    });
    install_with_card(cx, fixture.clone(), card.clone());
    (fixture, card)
}

fn install_with_card(cx: &mut Cx, transport: Arc<dyn SkillTransport>, card: SkillCard) {
    let transport = skill_transport_value(cx, transport).unwrap();
    let card = card.value(cx).unwrap();
    cx.call_function(&skill_install_symbol(), Args::new(vec![transport, card]))
        .unwrap();
}

fn as_runner(cx: &mut Cx, id: &str) -> Value {
    let target = cx.factory().string(id.to_owned()).unwrap();
    cx.call_function(&skill_as_runner_symbol(), Args::new(vec![target]))
        .unwrap()
}

fn realize_runner(cx: &mut Cx, runner: &Value, request: ModelRequest) -> ModelResponse {
    let fabric = runner.object().as_eval_fabric().unwrap();
    let reply = fabric
        .realize(
            cx,
            EvalRequest {
                expr: Expr::from(request),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    ModelResponse::try_from(reply.value.object().as_expr(cx).unwrap()).unwrap()
}

fn model_card(id: &str, transport_id: &str, operation: &str) -> SkillCard {
    SkillCard::fixture(FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Model Skill".to_owned(),
        description: "A model-role skill.".to_owned(),
        input_shape: model_args_shape(),
        output_shape: any_shape("model-result"),
        transport_id: transport_id.to_owned(),
        operation: operation.to_owned(),
    })
    .with_role(SkillRole::Model)
}

fn model_request(text: &str) -> ModelRequest {
    ModelRequest::new(Expr::String(text.to_owned()), Vec::new())
}

fn model_args_shape() -> ShapeRef {
    shape_value(
        Symbol::qualified("skill-runner-test", "model-args"),
        Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
    )
}

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("skill-runner-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

fn keyword(cx: &mut Cx, name: &str) -> Value {
    cx.factory().symbol(Symbol::new(name.to_owned())).unwrap()
}
