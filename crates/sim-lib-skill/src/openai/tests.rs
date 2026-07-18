use std::sync::Arc;

use serde_json::{Map, Value as JsonValue, json};
use sim_kernel::{Args, Cx, DefaultFactory, Expr, NoopEvalPolicy, ShapeRef, Symbol};
use sim_lib_openai_server::{
    DeterministicGatewayClock, GatewayEvent, GatewayRequest, MemoryGatewayStore, OpenAiTool,
    OpenAiToolRegistry, RESPONSES_PATH, ResponseIdGenerators, execute_response_request,
    install_openai_gateway_lib, openai_gateway_tools_capability,
};
use sim_shape::{ListShape, NumberValueShape, shape_value};

use crate::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, insert_skill_openai_tools,
    install_skill_lib, skill_call_capability, skill_install_capability, skill_install_symbol,
    skill_openai_tool_symbol, skill_openai_tools_symbol, skill_specific_call_capability,
    skill_transport_value,
};

#[test]
fn skill_openai_tool_descriptor_round_trips_through_openai_tool() {
    let mut cx = skill_gateway_cx(true, true);
    let _fixture = install_sum_skill(&mut cx);
    let descriptor = skill_descriptor_json(&mut cx);

    let tool = OpenAiTool::from_openai_descriptor(&descriptor).unwrap();

    assert_eq!(tool.openai_name(), "skill_math_add");
    assert_eq!(tool.symbol(), &Symbol::qualified("skill", "math-add"));
    assert!(descriptor["function"].get("x-sim-symbol").is_none());
    assert!(descriptor["function"].get("x-sim-capabilities").is_none());
    assert_eq!(
        descriptor["function"]["parameters"]["required"],
        json!(["arg0", "arg1"])
    );
    assert_eq!(
        descriptor["function"]["parameters"]["properties"]["arg0"]["type"],
        json!("number")
    );

    let listed = cx
        .call_function(&skill_openai_tools_symbol(), Args::default())
        .unwrap();
    let Expr::List(items) = listed.object().as_expr(&mut cx).unwrap() else {
        panic!("skill/openai-tools should return a descriptor list");
    };
    assert_eq!(items.len(), 1);

    let card = crate::skill_registry(&mut cx)
        .unwrap()
        .card_by_id("math.add")
        .unwrap()
        .unwrap();
    let mut registry = OpenAiToolRegistry::default();
    insert_skill_openai_tools(&mut cx, &mut registry, &[card]).unwrap();
    assert!(!registry.is_empty());
}

#[test]
fn responses_tool_flow_calls_fixture_skill_once_and_records_events() {
    let mut cx = skill_gateway_cx(true, true);
    let fixture = install_sum_skill(&mut cx);
    let descriptor = skill_descriptor_json(&mut cx);
    let execution = execute_skill_response(&mut cx, descriptor, json!({"arg0": 2, "arg1": 3}));
    let body = response_json(execution.response());

    assert_eq!(execution.response().status(), 200);
    assert!(body["output_text"].as_str().unwrap().contains('5'));
    assert_eq!(fixture.call_count(), 1);
    assert!(has_event(execution.events(), "tool-call"));
    assert!(has_event(execution.events(), "tool-result"));
}

#[test]
fn missing_openai_tool_capability_denies_before_skill_execution() {
    let mut cx = skill_gateway_cx(false, true);
    let fixture = install_sum_skill(&mut cx);
    let descriptor = skill_descriptor_json(&mut cx);
    let execution = execute_skill_response(&mut cx, descriptor, json!({"arg0": 2, "arg1": 3}));
    let body = response_json(execution.response());

    assert_eq!(execution.response().status(), 200);
    assert_eq!(fixture.call_count(), 0);
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("capability-denied")
    );
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("openai-gateway.tools")
    );
    assert!(has_event(execution.events(), "tool-result"));
}

#[test]
fn missing_skill_call_capability_denies_before_skill_execution() {
    let mut cx = skill_gateway_cx(true, false);
    let fixture = install_sum_skill(&mut cx);
    let descriptor = skill_descriptor_json(&mut cx);
    let execution = execute_skill_response(&mut cx, descriptor, json!({"arg0": 2, "arg1": 3}));
    let body = response_json(execution.response());

    assert_eq!(execution.response().status(), 200);
    assert_eq!(fixture.call_count(), 0);
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("capability-denied")
    );
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("skill.math.add.call")
    );
    assert!(has_event(execution.events(), "tool-result"));
}

fn skill_gateway_cx(openai_tools: bool, specific_skill_call: bool) -> Cx {
    let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    install_openai_gateway_lib(&mut cx).unwrap();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    if openai_tools {
        cx.grant(openai_gateway_tools_capability());
    }
    if specific_skill_call {
        cx.grant(skill_specific_call_capability("math.add"));
    }
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

fn skill_descriptor_json(cx: &mut Cx) -> JsonValue {
    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let value = cx
        .call_function(&skill_openai_tool_symbol(), Args::new(vec![target]))
        .unwrap();
    expr_to_json(&value.object().as_expr(cx).unwrap())
}

fn execute_skill_response(
    cx: &mut Cx,
    descriptor: JsonValue,
    arguments: JsonValue,
) -> sim_lib_openai_server::ResponseExecution {
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicGatewayClock::new(1_000, 10);
    let request = GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        serde_json::to_vec(&json!({
            "model": "fixture/tool-call",
            "input": arguments.to_string(),
            "store": true,
            "tools": [descriptor]
        }))
        .unwrap(),
    );
    execute_response_request(cx, &mut store, &mut ids, &mut clock, &request)
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

fn response_json(response: &sim_lib_openai_server::GatewayResponse) -> JsonValue {
    serde_json::from_slice(response.body()).unwrap()
}

fn has_event(events: &[GatewayEvent], kind: &str) -> bool {
    events
        .iter()
        .any(|event| event.kind().name.as_ref() == kind)
}

// This local projection matches the untagged descriptor shape used by these
// tests, which differs from sim_codec_json::project_expr_to_json.
fn expr_to_json(expr: &Expr) -> JsonValue {
    match expr {
        Expr::Nil => JsonValue::Null,
        Expr::Bool(value) => JsonValue::Bool(*value),
        Expr::Number(number) => serde_json::from_str(&number.canonical)
            .unwrap_or_else(|_| JsonValue::String(number.canonical.clone())),
        Expr::String(value) => JsonValue::String(value.clone()),
        Expr::Symbol(symbol) | Expr::Local(symbol) => JsonValue::String(symbol.as_qualified_str()),
        Expr::List(items) | Expr::Vector(items) => {
            JsonValue::Array(items.iter().map(expr_to_json).collect())
        }
        Expr::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(expr_key(key), expr_to_json(value));
            }
            JsonValue::Object(object)
        }
        other => JsonValue::String(format!("{other:?}")),
    }
}

fn expr_key(expr: &Expr) -> String {
    match expr {
        Expr::String(value) => value.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.as_qualified_str(),
        other => format!("{other:?}"),
    }
}
