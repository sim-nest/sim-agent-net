use std::sync::Arc;

use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, ShapeRef, Symbol};
use sim_shape::{AnyShape, shape_value};

use crate::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, SkillRole, install_skill_lib,
    skill_install_capability, skill_serve_capability, skill_specific_call_capability,
    skill_transport_value,
};

use super::skill_serve_mcp_symbol;

#[test]
fn serve_mcp_returns_filtered_in_process_projection() {
    let mut cx = skill_cx();
    cx.grant(skill_serve_capability());
    cx.grant(skill_install_capability());
    cx.grant(skill_specific_call_capability("skill.echo"));
    let fixture = Arc::new(fixture_transport());
    install_cards(
        &mut cx,
        fixture,
        vec![
            skill_card("skill.echo", SkillRole::Tool, "visible tool"),
            skill_card("skill.hidden", SkillRole::Tool, "private-token"),
            skill_card("skill.prompt", SkillRole::Prompt, "visible prompt"),
        ],
    );
    let options = Expr::Map(vec![
        field("allow-names", string_list(["skill.*"])),
        field("deny-names", string_list(["skill.hidden"])),
        field(
            "capabilities",
            string_list([skill_specific_call_capability("skill.echo").as_str()]),
        ),
    ]);
    let options = cx.factory().expr(options).unwrap();

    let result = cx
        .call_function(&skill_serve_mcp_symbol(), Args::new(vec![options]))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert_eq!(
        field_value(&result, "kind"),
        Some(&Expr::Symbol(Symbol::qualified("skill", "mcp-server")))
    );
    assert_eq!(
        strings_field(&result, "tools"),
        vec!["skill.echo".to_owned()]
    );
    assert_eq!(strings_field(&result, "prompts"), Vec::<String>::new());
    assert!(!format!("{result:?}").contains("private-token"));
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn install_cards(cx: &mut Cx, fixture: Arc<FixtureTransport>, cards: Vec<SkillCard>) {
    let mut values = vec![skill_transport_value(cx, fixture).unwrap()];
    values.extend(cards.into_iter().map(|card| card.value(cx).unwrap()));
    cx.call_function(&crate::skill_install_symbol(), Args::new(values))
        .unwrap();
}

fn fixture_transport() -> FixtureTransport {
    let fixture = FixtureTransport::new("serve-fixture");
    fixture.insert("echo", FixtureBehavior::EchoArgs).unwrap();
    fixture
}

fn skill_card(id: &str, role: SkillRole, description: &str) -> SkillCard {
    let mut card = SkillCard::fixture(FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: id.to_owned(),
        description: description.to_owned(),
        input_shape: any_shape("args"),
        output_shape: any_shape("result"),
        transport_id: "serve-fixture".to_owned(),
        operation: "echo".to_owned(),
    });
    card.roles = vec![role];
    card
}

fn strings_field(expr: &Expr, name: &str) -> Vec<String> {
    match field_value(expr, name) {
        Some(Expr::List(items)) => items
            .iter()
            .filter_map(|expr| match expr {
                Expr::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

use sim_value::access::field_any as field_value;

fn string_list<'a>(items: impl IntoIterator<Item = &'a str>) -> Expr {
    Expr::List(
        items
            .into_iter()
            .map(|item| Expr::String(item.to_owned()))
            .collect(),
    )
}

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("skill-serve-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_value::build::entry as field;
