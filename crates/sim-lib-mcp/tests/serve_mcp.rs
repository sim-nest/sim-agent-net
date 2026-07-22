#![cfg(feature = "skill")]

// conformance: MCP server surface boots and serves through the command entry point.

use std::sync::Arc;

use sim_codec_mcp::{CAPABILITY_DENIED, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse};
use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, ShapeRef, Symbol};
use sim_lib_mcp::{McpProfile, McpRouter, McpSession, mcp_tools_call_capability};
use sim_lib_skill::{
    FixtureBehavior, FixtureSkillSpec, FixtureTransport, SkillCard, SkillRole, install_skill_lib,
    skill_call_capability, skill_install_capability, skill_install_symbol,
    skill_specific_call_capability, skill_transport_value,
};
use sim_shape::{AnyShape, shape_value};

#[test]
fn in_process_mcp_projection_lists_by_role_and_capability() {
    let mut cx = skill_cx();
    bind_role_fixture_cards(&mut cx);
    let session = McpSession::new("skill-serve-list", McpProfile::all())
        .with_granted_capability(skill_specific_call_capability("skill.echo"))
        .with_granted_capability(skill_specific_call_capability("skill.resource"))
        .with_granted_capability(skill_specific_call_capability("skill.prompt"));
    let mut router = McpRouter::new(session);

    let tools = expect_response_result(list_request(&mut cx, &mut router, "tools/list"));
    let resources = expect_response_result(list_request(&mut cx, &mut router, "resources/list"));
    let prompts = expect_response_result(list_request(&mut cx, &mut router, "prompts/list"));

    assert_eq!(tool_names(&tools), vec!["skill.echo"]);
    assert_eq!(resource_uris(&resources), vec!["skill://skill.resource"]);
    assert_eq!(prompt_names(&prompts), vec!["skill.prompt"]);
    let output = format!("{tools:?}{resources:?}{prompts:?}");
    assert!(!output.contains("skill.hidden"));
    assert!(!output.contains("private-token"));
}

#[test]
fn in_process_mcp_tools_call_uses_skill_callable_once() {
    let mut cx = skill_cx();
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("skill.echo"));
    let fixture = bind_fixture_cards(
        &mut cx,
        vec![skill_card("skill.echo", SkillRole::Tool, "visible tool")],
    );
    let session = McpSession::new("skill-serve-call", McpProfile::all())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(skill_specific_call_capability("skill.echo"));
    let mut router = McpRouter::new(session);

    let result = expect_response_result(tools_call(
        &mut cx,
        &mut router,
        "skill.echo",
        vec![Expr::String("hello".to_owned())],
    ));

    assert_eq!(fixture.call_count(), 1);
    assert_eq!(is_error(&result), Some(false));
    assert_eq!(
        single_json_content(&result),
        Some(&Expr::List(vec![Expr::String("hello".to_owned())]))
    );
}

#[test]
fn tools_call_denial_returns_mcp_error_without_leaking_skill_metadata() {
    let mut cx = skill_cx();
    cx.grant(skill_call_capability());
    let fixture = bind_fixture_cards(
        &mut cx,
        vec![skill_card("skill.hidden", SkillRole::Tool, "private-token")],
    );
    let session = McpSession::new("skill-serve-denied", McpProfile::all())
        .with_granted_capability(mcp_tools_call_capability());
    let mut router = McpRouter::new(session);

    let error = expect_error(tools_call(
        &mut cx,
        &mut router,
        "skill.hidden",
        vec![Expr::String("blocked".to_owned())],
    ));

    assert_eq!(error.error.code, CAPABILITY_DENIED);
    assert!(!format!("{error:?}").contains("private-token"));
    assert_eq!(fixture.call_count(), 0);
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn bind_role_fixture_cards(cx: &mut Cx) {
    bind_fixture_cards(
        cx,
        vec![
            skill_card("skill.echo", SkillRole::Tool, "visible tool"),
            skill_card("skill.hidden", SkillRole::Tool, "private-token"),
            skill_card("skill.resource", SkillRole::Resource, "visible resource"),
            skill_card(
                "skill.hidden-resource",
                SkillRole::Resource,
                "private-token",
            ),
            skill_card("skill.prompt", SkillRole::Prompt, "visible prompt"),
            skill_card("skill.hidden-prompt", SkillRole::Prompt, "private-token"),
        ],
    );
}

fn bind_fixture_cards(cx: &mut Cx, cards: Vec<SkillCard>) -> Arc<FixtureTransport> {
    let fixture = Arc::new(fixture_transport());
    install_cards(cx, fixture.clone(), cards);
    fixture
}

fn install_cards(cx: &mut Cx, fixture: Arc<FixtureTransport>, cards: Vec<SkillCard>) {
    cx.grant(skill_install_capability());
    let mut values = vec![skill_transport_value(cx, fixture).unwrap()];
    values.extend(cards.into_iter().map(|card| card.value(cx).unwrap()));
    cx.call_function(&skill_install_symbol(), Args::new(values))
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

fn list_request(cx: &mut Cx, router: &mut McpRouter, method: &str) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(method.to_owned()),
                method: method.to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap()
        .unwrap()
}

fn tools_call(
    cx: &mut Cx,
    router: &mut McpRouter,
    name: &str,
    arguments: Vec<Expr>,
) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(format!("call-{name}")),
                method: "tools/call".to_owned(),
                params: Expr::Map(vec![
                    field("name", Expr::String(name.to_owned())),
                    field("arguments", Expr::List(arguments)),
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

fn tool_names(result: &Expr) -> Vec<&str> {
    list_field(result, "tools")
        .iter()
        .filter_map(|tool| string_field(tool, "name"))
        .collect()
}

fn resource_uris(result: &Expr) -> Vec<&str> {
    list_field(result, "resources")
        .iter()
        .filter_map(|resource| string_field(resource, "uri"))
        .collect()
}

fn prompt_names(result: &Expr) -> Vec<&str> {
    list_field(result, "prompts")
        .iter()
        .filter_map(|prompt| string_field(prompt, "name"))
        .collect()
}

fn list_field<'a>(expr: &'a Expr, name: &str) -> &'a [Expr] {
    match field_value(expr, name) {
        Some(Expr::List(items)) => items,
        _ => &[],
    }
}

fn is_error(result: &Expr) -> Option<bool> {
    match field_value(result, "isError") {
        Some(Expr::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn single_json_content(result: &Expr) -> Option<&Expr> {
    single_content(result).and_then(|content| field_value(content, "json"))
}

fn single_content(result: &Expr) -> Option<&Expr> {
    match field_value(result, "content") {
        Some(Expr::List(items)) if items.len() == 1 => items.first(),
        _ => None,
    }
}

fn string_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a str> {
    field_value(expr, name).and_then(|value| match value {
        Expr::String(text) => Some(text.as_str()),
        _ => None,
    })
}

fn field_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(fields) = expr else {
        return None;
    };
    fields.iter().find_map(|(key, value)| {
        let key = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (key == name).then_some(value)
    })
}

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("skill-serve-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_value::build::entry as field;
