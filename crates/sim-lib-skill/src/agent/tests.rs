use std::sync::Arc;

use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, HandleSeed, ShapeRef, Symbol};
use sim_shape::{ListShape, NumberValueShape, shape_value};

use crate::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, install_skill_lib,
    skill_as_tool_symbol, skill_call_capability, skill_install_capability,
    skill_specific_call_capability, skill_transport_value,
};

#[test]
fn skill_as_tool_returns_existing_agent_tool_projection() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    let _fixture = install_sum_skill(&mut cx);
    let target = cx.factory().string("math.add".to_owned()).unwrap();

    let value = cx
        .call_function(&skill_as_tool_symbol(), Args::new(vec![target]))
        .unwrap();
    let tool = value
        .object()
        .downcast_ref::<sim_lib_agent::Tool>()
        .unwrap();

    assert_eq!(tool.symbol, Symbol::qualified("skill", "math.add"));
    assert_eq!(tool.description, "Add two numbers with a fixture skill.");
    assert_eq!(tool.category, Symbol::new("skill"));
    assert_eq!(
        tool.capabilities,
        vec![skill_specific_call_capability("math.add")]
    );
}

#[test]
fn skill_tool_call_uses_bound_skill_callable_once() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("math.add"));
    let fixture = install_sum_skill(&mut cx);
    let target = cx.factory().string("math.add".to_owned()).unwrap();
    let tool = cx
        .call_function(&skill_as_tool_symbol(), Args::new(vec![target]))
        .unwrap();
    let left = number_value(&mut cx, 2);
    let right = number_value(&mut cx, 3);

    let result = cx.call_value(tool, Args::new(vec![left, right])).unwrap();

    assert_eq!(number_expr(&mut cx, result), "5");
    assert_eq!(fixture.call_count(), 1);
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x5A11_0004),
    );
    install_skill_lib(&mut cx).unwrap();
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();
    cx
}

fn install_sum_skill(cx: &mut Cx) -> Arc<FixtureTransport> {
    let fixture = Arc::new(FixtureTransport::new("math"));
    fixture.insert("add", FixtureBehavior::SumNumbers).unwrap();
    let transport = skill_transport_value(cx, fixture.clone()).unwrap();
    let card = sum_card().value(cx).unwrap();
    cx.call_function(
        &crate::skill_install_symbol(),
        Args::new(vec![transport, card]),
    )
    .unwrap();
    fixture
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

fn number_value(cx: &mut Cx, value: u32) -> sim_kernel::Value {
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), value.to_string())
        .unwrap()
}

fn number_expr(cx: &mut Cx, value: sim_kernel::Value) -> String {
    match value.object().as_expr(cx).unwrap() {
        Expr::Number(number) => number.canonical,
        other => panic!("expected number, got {other:?}"),
    }
}
