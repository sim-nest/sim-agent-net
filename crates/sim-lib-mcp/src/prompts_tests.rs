use std::sync::Arc;

use sim_codec_mcp::{INVALID_PARAMS, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse};
use sim_kernel::{Args, CapabilityName, Cx, Expr, ShapeRef, Symbol};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpAnnotation, McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession,
    install_mcp_lib, mcp_prompts_get_capability, prompts_symbol,
};

#[test]
fn prompts_list_filters_roles_capabilities_and_redacts() {
    let mut cx = cx();
    let visible = CapabilityName::new("fixture.prompt.visible");
    let session = McpSession::new("prompts", McpProfile::all())
        .with_granted_capability(visible.clone())
        .with_native_cards(vec![
            native_prompt("visible.prompt", vec![visible]),
            native_prompt(
                "blocked.prompt",
                vec![CapabilityName::new("fixture.prompt.blocked")],
            ),
            native_resource("visible.resource"),
        ]);
    let mut router = McpRouter::new(session);

    let result = prompts_list_result(&mut cx, &mut router);
    let text = format!("{result:?}");

    assert_eq!(prompt_names(&result), vec!["visible.prompt"]);
    assert!(!text.contains("fixture.prompt.visible"));
    assert!(!text.contains("private-token"));
}

#[test]
fn prompts_get_native_prompt_does_not_call_subject() {
    let mut cx = cx();
    let prompt_cap = CapabilityName::new("fixture.prompt.get");
    let card = native_prompt("safe.prompt", vec![prompt_cap.clone()]);
    let session = McpSession::new("prompts", McpProfile::all())
        .with_granted_capability(mcp_prompts_get_capability())
        .with_granted_capability(prompt_cap)
        .with_native_cards(vec![card]);
    let mut router = McpRouter::new(session);

    let result =
        expect_response_result(prompts_get(&mut cx, &mut router, "safe.prompt", Vec::new()));

    let json = single_json_content(&result).expect("prompt get should return json content");
    assert_eq!(
        field_value(json, "name"),
        Some(&Expr::String("safe.prompt".to_owned()))
    );
    assert!(!format!("{result:?}").contains("private-token"));
}

#[test]
fn prompts_get_unknown_prompt_returns_structured_not_found() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();

    let error = expect_error(prompts_get(
        &mut cx,
        &mut router,
        "missing.prompt",
        Vec::new(),
    ));

    assert_eq!(error.error.code, INVALID_PARAMS);
    assert_eq!(
        field_value(&error.error.data, "code"),
        Some(&Expr::String("not-found".to_owned()))
    );
    assert_eq!(
        field_value(&error.error.data, "kind"),
        Some(&Expr::String("prompt".to_owned()))
    );
    assert_eq!(
        field_value(&error.error.data, "id"),
        Some(&Expr::String("missing.prompt".to_owned()))
    );
}

#[test]
fn prompts_lisp_function_returns_empty_fixture_list() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();

    let result = cx
        .call_function(&prompts_symbol(), Args::default())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert_eq!(prompt_names(&result), Vec::<String>::new());
}

#[cfg(feature = "skill")]
#[test]
fn prompts_get_skill_prompt_uses_bound_skill_callable_once() {
    let mut cx = skill_cx();
    cx.grant(sim_lib_skill::skill_call_capability());
    cx.grant(sim_lib_skill::skill_specific_call_capability(
        "skill.prompt",
    ));
    let fixture = bind_skill(
        &mut cx,
        skill_card("skill.prompt", sim_lib_skill::SkillRole::Prompt),
    );
    let session = McpSession::new("skill-prompt", McpProfile::all())
        .with_granted_capability(mcp_prompts_get_capability())
        .with_granted_capability(sim_lib_skill::skill_specific_call_capability(
            "skill.prompt",
        ));
    let mut router = McpRouter::new(session);

    let result = expect_response_result(prompts_get(
        &mut cx,
        &mut router,
        "skill.prompt",
        vec![Expr::String("hello".to_owned())],
    ));

    assert_eq!(fixture.call_count(), 1);
    assert_eq!(
        single_json_content(&result),
        Some(&Expr::List(vec![Expr::String("hello".to_owned())]))
    );
}

fn prompts_list_result(cx: &mut Cx, router: &mut McpRouter) -> Expr {
    expect_response_result(
        router
            .handle(
                cx,
                McpEnvelope::Request(McpRequest {
                    id: Expr::String("prompts".to_owned()),
                    method: "prompts/list".to_owned(),
                    params: Expr::Nil,
                }),
            )
            .unwrap()
            .unwrap(),
    )
}

fn prompts_get(
    cx: &mut Cx,
    router: &mut McpRouter,
    name: &str,
    arguments: Vec<Expr>,
) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(format!("get-{name}")),
                method: "prompts/get".to_owned(),
                params: Expr::Map(vec![
                    field("name", Expr::String(name.to_owned())),
                    field("arguments", Expr::List(arguments)),
                ]),
            }),
        )
        .unwrap()
        .unwrap()
}

fn native_prompt(name: &str, capabilities: Vec<CapabilityName>) -> McpNativeCard {
    let mut card = McpNativeCard::new(
        Symbol::qualified("fixture", format!("missing-{name}")),
        "Native prompt",
    )
    .with_shapes(any_shape("args"), any_shape("result"))
    .exported(
        McpExportFacet::prompt()
            .with_name(name.to_owned())
            .with_annotation(McpAnnotation::private(
                Symbol::new("secret"),
                Expr::String("private-token".to_owned()),
            )),
    );
    for capability in capabilities {
        card = card.with_capability(capability);
    }
    card
}

fn native_resource(name: &str) -> McpNativeCard {
    McpNativeCard::new(
        Symbol::qualified("fixture", name.to_owned()),
        "Native resource",
    )
    .with_shapes(any_shape("args"), any_shape("result"))
    .exported(
        McpExportFacet::resource()
            .with_name(name.to_owned())
            .with_uri(format!("sim://fixture/{name}")),
    )
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

fn prompt_names(result: &Expr) -> Vec<String> {
    prompts(result)
        .iter()
        .filter_map(|prompt| field_value(prompt, "name"))
        .filter_map(|value| match value {
            Expr::String(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn prompts(result: &Expr) -> &[Expr] {
    let Some(Expr::List(prompts)) = field_value(result, "prompts") else {
        panic!("prompts/list result should contain prompts list");
    };
    prompts
}

fn single_json_content(result: &Expr) -> Option<&Expr> {
    let Some(Expr::List(messages)) = field_value(result, "messages") else {
        return None;
    };
    let [message] = messages.as_slice() else {
        return None;
    };
    field_value(message, "content").and_then(|content| field_value(content, "json"))
}

use sim_kernel::testing::eager_cx as cx;
use sim_value::access::field_any as field_value;

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_value::build::entry as field;

#[cfg(feature = "skill")]
fn skill_cx() -> Cx {
    let mut cx = cx();
    sim_lib_skill::install_skill_lib(&mut cx).unwrap();
    cx
}

#[cfg(feature = "skill")]
fn skill_card(id: &str, role: sim_lib_skill::SkillRole) -> sim_lib_skill::SkillCard {
    let mut card = sim_lib_skill::SkillCard::fixture(sim_lib_skill::FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Fixture Skill".to_owned(),
        description: "A skill projected through prompts/get.".to_owned(),
        input_shape: any_shape("skill-args"),
        output_shape: any_shape("skill-result"),
        transport_id: "prompts-transport".to_owned(),
        operation: "echo".to_owned(),
    });
    card.roles = vec![role];
    card
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx, card: sim_lib_skill::SkillCard) -> Arc<sim_lib_skill::FixtureTransport> {
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("prompts-transport"));
    fixture
        .insert("echo", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    registry.install_transport(fixture.clone()).unwrap();
    registry.bind_card(cx, card).unwrap();
    fixture
}
