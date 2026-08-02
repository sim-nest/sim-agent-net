use std::sync::Arc;

use serde_json::{Value as JsonValue, json};
use sim_codec_mcp::{CAPABILITY_DENIED, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse};
use sim_kernel::{Args, Cx, DefaultFactory, Expr, NoopEvalPolicy, ShapeRef, Symbol, Value};
use sim_lib_openai_server::{
    DeterministicWallClock, GatewayEvent, GatewayRequest, MemoryGatewayStore, OpenAiTool,
    RESPONSES_PATH, ResponseIdGenerators, execute_response_request, install_openai_gateway_lib,
    openai_gateway_tools_capability,
};
use sim_shape::{ListShape, NumberValueShape, shape_value};

use crate::{McpProfile, McpRouter, McpSession, mcp_tools_call_capability, skill_surface_rows};

const SKILL_ID: &str = "math.add";
const OPENAI_TOOL_NAME: &str = "skill_math_add";

#[test]
fn skill_callable_is_shared_by_direct_agent_openai_and_mcp_paths() {
    let mut cx = coexistence_cx(true);
    let (fixture, card) = bind_sum_skill(&mut cx);
    let symbol = skill_symbol();

    let direct = direct_skill_call(&mut cx).unwrap();
    assert_eq!(number_canonical(&mut cx, direct), "5");
    assert_eq!(fixture.call_count(), 1);

    let agent_tool = agent_tool_value(&mut cx);
    {
        let tool = agent_tool
            .object()
            .downcast_ref::<sim_lib_agent::Tool>()
            .unwrap();
        assert_eq!(tool.symbol, symbol);
        assert_eq!(tool.capabilities, card.capabilities);
    }
    let args = number_args(&mut cx);
    let agent = cx.call_value(agent_tool, Args::new(args)).unwrap();
    assert_eq!(number_canonical(&mut cx, agent), "5");
    assert_eq!(fixture.call_count(), 2);

    let descriptor = openai_descriptor(&mut cx, &card);
    let openai_tool = OpenAiTool::from_openai_descriptor(&descriptor).unwrap();
    assert_eq!(openai_tool.openai_name(), OPENAI_TOOL_NAME);
    assert_eq!(
        openai_tool.symbol(),
        &Symbol::qualified("skill", "math-add")
    );
    assert!(descriptor["function"].get("x-sim-symbol").is_none());
    assert!(descriptor["function"].get("x-sim-capabilities").is_none());
    let openai = execute_openai_response(&mut cx, descriptor, json!({"arg0": 2, "arg1": 3}));
    assert_eq!(openai.response().status(), 200);
    assert!(
        response_json(openai.response())["output_text"]
            .as_str()
            .unwrap()
            .contains('5')
    );
    assert!(has_event(openai.events(), "tool-call"));
    assert!(has_event(openai.events(), "tool-result"));
    assert_eq!(fixture.call_count(), 3);

    let mut router = allowed_router();
    let listed = mcp_tools_list(&mut cx, &mut router);
    assert_eq!(tool_names(&listed), vec![SKILL_ID.to_owned()]);
    let row = skill_surface_rows(&mut cx)
        .unwrap()
        .into_iter()
        .find(|row| row.name == SKILL_ID)
        .unwrap();
    assert_eq!(row.symbol.as_ref(), Some(&symbol));

    let called = expect_response_result(mcp_tools_call(&mut cx, &mut router));
    assert_eq!(single_json_content(&called), Some(&number_expr(5)));
    assert_eq!(fixture.call_count(), 4);
}

#[test]
fn skill_policy_denial_is_consistent_across_direct_agent_openai_and_mcp() {
    let mut cx = coexistence_cx(false);
    let (fixture, card) = bind_sum_skill(&mut cx);

    let direct = direct_skill_call(&mut cx).unwrap_err();
    assert!(format!("{direct}").contains("skill.math.add.call"));

    let agent_tool = agent_tool_value(&mut cx);
    let args = number_args(&mut cx);
    let agent = cx.call_value(agent_tool, Args::new(args)).unwrap_err();
    assert!(format!("{agent}").contains("skill.math.add.call"));

    let descriptor = openai_descriptor(&mut cx, &card);
    let openai = execute_openai_response(&mut cx, descriptor, json!({"arg0": 2, "arg1": 3}));
    let text = response_json(openai.response())["output_text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains("capability-denied"));
    assert!(text.contains("skill.math.add.call"));

    let mut router = denied_router();
    let error = expect_error(mcp_tools_call(&mut cx, &mut router));
    assert_eq!(error.error.code, CAPABILITY_DENIED);

    assert_eq!(fixture.call_count(), 0);
}

fn coexistence_cx(grant_specific_skill: bool) -> Cx {
    let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    sim_lib_skill::install_skill_lib(&mut cx).unwrap();
    install_openai_gateway_lib(&mut cx).unwrap();
    cx.grant(sim_lib_skill::skill_call_capability());
    cx.grant(openai_gateway_tools_capability());
    if grant_specific_skill {
        cx.grant(sim_lib_skill::skill_specific_call_capability(SKILL_ID));
    }
    cx
}

fn bind_sum_skill(
    cx: &mut Cx,
) -> (
    Arc<sim_lib_skill::FixtureTransport>,
    sim_lib_skill::SkillCard,
) {
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("math"));
    fixture
        .insert("add", sim_lib_skill::FixtureBehavior::SumNumbers)
        .unwrap();
    let card = sum_card();
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    registry.install_transport(fixture.clone()).unwrap();
    registry.bind_card(cx, card.clone()).unwrap();
    (fixture, card)
}

fn sum_card() -> sim_lib_skill::SkillCard {
    sim_lib_skill::SkillCard::fixture(sim_lib_skill::FixtureSkillSpec {
        id: SKILL_ID.to_owned(),
        symbol: skill_symbol(),
        title: "Add Numbers".to_owned(),
        description: "Add two numbers with a fixture skill.".to_owned(),
        input_shape: sum_args_shape(),
        output_shape: number_shape("sum-result"),
        transport_id: "math".to_owned(),
        operation: "add".to_owned(),
    })
}

fn direct_skill_call(cx: &mut Cx) -> sim_kernel::Result<Value> {
    let target = cx.factory().string(SKILL_ID.to_owned())?;
    let mut args = vec![target];
    args.extend(number_args(cx));
    cx.call_function(&sim_lib_skill::skill_call_symbol(), Args::new(args))
}

fn agent_tool_value(cx: &mut Cx) -> Value {
    let target = cx.factory().string(SKILL_ID.to_owned()).unwrap();
    cx.call_function(
        &sim_lib_skill::skill_as_tool_symbol(),
        Args::new(vec![target]),
    )
    .unwrap()
}

fn openai_descriptor(cx: &mut Cx, card: &sim_lib_skill::SkillCard) -> JsonValue {
    sim_lib_skill::skill_openai_tool_descriptor(cx, card).unwrap()
}

fn execute_openai_response(
    cx: &mut Cx,
    descriptor: JsonValue,
    arguments: JsonValue,
) -> sim_lib_openai_server::ResponseExecution {
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicWallClock::new(1_000, 10);
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

fn allowed_router() -> McpRouter {
    McpRouter::new(
        McpSession::new("coexistence", McpProfile::all())
            .with_granted_capability(mcp_tools_call_capability())
            .with_granted_capability(sim_lib_skill::skill_specific_call_capability(SKILL_ID)),
    )
}

fn denied_router() -> McpRouter {
    McpRouter::new(
        McpSession::new("coexistence-denied", McpProfile::all())
            .with_granted_capability(mcp_tools_call_capability()),
    )
}

fn mcp_tools_list(cx: &mut Cx, router: &mut McpRouter) -> Expr {
    let response = router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String("coexistence-list".to_owned()),
                method: "tools/list".to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap()
        .unwrap();
    expect_response_result(response)
}

fn mcp_tools_call(cx: &mut Cx, router: &mut McpRouter) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String("coexistence-call".to_owned()),
                method: "tools/call".to_owned(),
                params: Expr::Map(vec![
                    field("name", Expr::String(SKILL_ID.to_owned())),
                    field(
                        "arguments",
                        Expr::List(vec![number_expr(2), number_expr(3)]),
                    ),
                ]),
            }),
        )
        .unwrap()
        .unwrap()
}

fn expect_response_result(envelope: McpEnvelope) -> Expr {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected MCP response envelope");
    };
    result
}

fn expect_error(envelope: McpEnvelope) -> McpErrorEnvelope {
    let McpEnvelope::Error(error) = envelope else {
        panic!("expected MCP error envelope");
    };
    error
}

fn tool_names(result: &Expr) -> Vec<String> {
    let Some(Expr::List(tools)) = field_value(result, "tools") else {
        panic!("tools/list result should contain tools");
    };
    tools
        .iter()
        .filter_map(|tool| field_value(tool, "name"))
        .filter_map(|value| match value {
            Expr::String(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn single_json_content(result: &Expr) -> Option<&Expr> {
    let Some(Expr::List(content)) = field_value(result, "content") else {
        return None;
    };
    let [item] = content.as_slice() else {
        return None;
    };
    field_value(item, "json")
}

use sim_value::access::field_any as field_value;

fn response_json(response: &sim_lib_openai_server::GatewayResponse) -> JsonValue {
    serde_json::from_slice(response.body()).unwrap()
}

fn has_event(events: &[GatewayEvent], kind: &str) -> bool {
    events
        .iter()
        .any(|event| event.kind().name.as_ref() == kind)
}

fn number_args(cx: &mut Cx) -> Vec<Value> {
    vec![number_value(cx, 2), number_value(cx, 3)]
}

fn number_value(cx: &mut Cx, value: u32) -> Value {
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), value.to_string())
        .unwrap()
}

fn number_expr(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn number_canonical(cx: &mut Cx, value: Value) -> String {
    match value.object().as_expr(cx).unwrap() {
        Expr::Number(number) => number.canonical,
        other => panic!("expected number, got {other:?}"),
    }
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

fn skill_symbol() -> Symbol {
    Symbol::qualified("skill", SKILL_ID)
}

use sim_value::build::entry as field;
