use std::sync::Arc;

use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, Symbol, Value};

use crate::{
    FixtureBehavior, FixtureMcpTransport, McpCallParams, McpToolDescriptor, SkillRole,
    SkillTransport, install_skill_lib, skill_call_capability, skill_call_symbol,
    skill_install_capability, skill_install_symbol, skill_specific_call_capability,
    skill_transport_value,
};

use super::{McpToolResult, skill_mcp_call_symbol, skill_mcp_tools_symbol};

#[test]
fn fixture_mcp_discovery_binds_tool_role_skill() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = Arc::new(fixture_mcp_transport());
    let cards = fixture.discover(&mut cx).unwrap();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].roles.contains(&SkillRole::Tool));

    install_cards(&mut cx, fixture.clone(), cards);

    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let left = number_value(&mut cx, 2);
    let right = number_value(&mut cx, 3);
    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, left, right]))
        .unwrap();

    assert_eq!(number_expr(&mut cx, result), "5");
    assert_eq!(fixture.call_count(), 1);
}

#[test]
fn skill_mcp_tools_returns_descriptor_data_generated_from_skill_card() {
    let mut cx = installed_mcp_cx();
    let value = cx
        .call_function(&skill_mcp_tools_symbol(), Args::default())
        .unwrap();
    let Expr::List(items) = value.object().as_expr(&mut cx).unwrap() else {
        panic!("skill/mcp-tools should return descriptor list");
    };
    assert_eq!(items.len(), 1);
    let descriptor = McpToolDescriptor::from_expr(&items[0]).unwrap();
    assert_eq!(descriptor.name, "math.add");
    assert_eq!(descriptor.title.as_deref(), Some("Add Numbers"));
    assert_eq!(descriptor.description, "Add numbers through fixture MCP.");
}

#[test]
fn skill_mcp_call_executes_the_same_skill_callable_as_skill_call() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = Arc::new(fixture_mcp_transport());
    let cards = fixture.discover(&mut cx).unwrap();
    install_cards(&mut cx, fixture.clone(), cards);

    let params = McpCallParams {
        name: "math.add".to_owned(),
        arguments: vec![number_expr_literal(4), number_expr_literal(6)],
    };
    let params = cx.factory().expr(params.to_expr()).unwrap();
    let result = cx
        .call_function(&skill_mcp_call_symbol(), Args::new(vec![params]))
        .unwrap();
    let result = McpToolResult::from_expr(&result.object().as_expr(&mut cx).unwrap()).unwrap();
    assert_eq!(result.result, number_expr_literal(10));
    assert!(!result.is_error);

    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let left = number_value(&mut cx, 1);
    let right = number_value(&mut cx, 2);
    let direct = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, left, right]))
        .unwrap();

    assert_eq!(number_expr(&mut cx, direct), "3");
    assert_eq!(fixture.call_count(), 2);
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn installed_mcp_cx() -> Cx {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let fixture = Arc::new(fixture_mcp_transport());
    let cards = fixture.discover(&mut cx).unwrap();
    install_cards(&mut cx, fixture, cards);
    cx
}

fn fixture_mcp_transport() -> FixtureMcpTransport {
    let fixture = FixtureMcpTransport::new("mcp-fixture");
    fixture
        .insert_tool(
            McpToolDescriptor {
                name: "math.add".to_owned(),
                title: Some("Add Numbers".to_owned()),
                description: "Add numbers through fixture MCP.".to_owned(),
                input_schema: sum_args_schema(),
                output_schema: Some(number_schema()),
            },
            FixtureBehavior::SumNumbers,
        )
        .unwrap();
    fixture
}

fn install_cards(cx: &mut Cx, fixture: Arc<FixtureMcpTransport>, cards: Vec<crate::SkillCard>) {
    let mut values = vec![skill_transport_value(cx, fixture).unwrap()];
    values.extend(cards.into_iter().map(|card| card.value(cx).unwrap()));
    cx.call_function(&skill_install_symbol(), Args::new(values))
        .unwrap();
}

fn sum_args_schema() -> Expr {
    Expr::List(vec![number_schema(), number_schema()])
}

fn number_schema() -> Expr {
    Expr::Symbol(Symbol::qualified("core", "Number"))
}

fn number_value(cx: &mut Cx, value: u32) -> Value {
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), value.to_string())
        .unwrap()
}

fn number_expr_literal(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn number_expr(cx: &mut Cx, value: Value) -> String {
    match value.object().as_expr(cx).unwrap() {
        Expr::Number(number) => number.canonical,
        other => panic!("expected number, got {other:?}"),
    }
}
