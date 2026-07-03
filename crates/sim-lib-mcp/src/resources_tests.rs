use std::sync::Arc;

use sim_codec_mcp::{INVALID_PARAMS, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse};
use sim_kernel::{Args, CapabilityName, Cx, Expr, ShapeRef, Symbol};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpAnnotation, McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession,
    install_mcp_lib, mcp_resources_read_capability, resources_symbol,
};

#[test]
fn resources_list_filters_roles_capabilities_and_redacts() {
    let mut cx = cx();
    let visible = CapabilityName::new("fixture.resource.visible");
    let session = McpSession::new("resources", McpProfile::all())
        .with_granted_capability(visible.clone())
        .with_native_cards(vec![
            native_resource(
                "visible.resource",
                "sim://browse/fixture/visible",
                vec![visible],
            ),
            native_resource(
                "blocked.resource",
                "sim://browse/fixture/blocked",
                vec![CapabilityName::new("fixture.resource.blocked")],
            ),
            native_resource("bad.resource", "file://fixture/bad", Vec::new()),
            native_tool("visible.tool"),
        ]);
    let mut router = McpRouter::new(session);

    let result = resources_list_result(&mut cx, &mut router);
    let text = format!("{result:?}");

    assert_eq!(resource_uris(&result), vec!["sim://browse/fixture/visible"]);
    assert!(!text.contains("fixture.resource.visible"));
    assert!(!text.contains("private-token"));
}

#[test]
fn resources_read_native_sim_resource_does_not_call_subject() {
    let mut cx = cx();
    let read_cap = CapabilityName::new("fixture.resource.read");
    let card = native_resource(
        "safe.resource",
        "sim://browse/fixture/safe",
        vec![read_cap.clone()],
    );
    let session = McpSession::new("resources", McpProfile::all())
        .with_granted_capability(mcp_resources_read_capability())
        .with_granted_capability(read_cap)
        .with_native_cards(vec![card]);
    let mut router = McpRouter::new(session);

    let result = expect_response_result(resources_read(
        &mut cx,
        &mut router,
        "sim://browse/fixture/safe",
    ));

    let json = single_json_content(&result).expect("resource read should return json content");
    assert_eq!(
        field_value(json, "name"),
        Some(&Expr::String("safe.resource".to_owned()))
    );
    assert!(!format!("{result:?}").contains("private-token"));
}

#[test]
fn resources_read_unknown_uri_returns_structured_not_found() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();

    let error = expect_error(resources_read(
        &mut cx,
        &mut router,
        "sim://browse/fixture/missing",
    ));

    assert_eq!(error.error.code, INVALID_PARAMS);
    assert_eq!(
        field_value(&error.error.data, "code"),
        Some(&Expr::String("not-found".to_owned()))
    );
    assert_eq!(
        field_value(&error.error.data, "kind"),
        Some(&Expr::String("resource".to_owned()))
    );
    assert_eq!(
        field_value(&error.error.data, "id"),
        Some(&Expr::String("sim://browse/fixture/missing".to_owned()))
    );
}

#[test]
fn resources_lisp_function_returns_empty_fixture_list() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();

    let result = cx
        .call_function(&resources_symbol(), Args::default())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert_eq!(resource_uris(&result), Vec::<String>::new());
}

#[cfg(feature = "skill")]
#[test]
fn resources_read_skill_resource_uses_bound_skill_callable_once() {
    let mut cx = skill_cx();
    cx.grant(sim_lib_skill::skill_call_capability());
    cx.grant(sim_lib_skill::skill_specific_call_capability(
        "skill.resource",
    ));
    let fixture = bind_skill(
        &mut cx,
        skill_card("skill.resource", sim_lib_skill::SkillRole::Resource),
    );
    let session = McpSession::new("skill-resource", McpProfile::all())
        .with_granted_capability(mcp_resources_read_capability())
        .with_granted_capability(sim_lib_skill::skill_specific_call_capability(
            "skill.resource",
        ));
    let mut router = McpRouter::new(session);

    let result = expect_response_result(resources_read(
        &mut cx,
        &mut router,
        "skill://skill.resource",
    ));

    assert_eq!(fixture.call_count(), 1);
    assert_eq!(single_json_content(&result), Some(&Expr::List(Vec::new())));
}

fn resources_list_result(cx: &mut Cx, router: &mut McpRouter) -> Expr {
    expect_response_result(
        router
            .handle(
                cx,
                McpEnvelope::Request(McpRequest {
                    id: Expr::String("resources".to_owned()),
                    method: "resources/list".to_owned(),
                    params: Expr::Nil,
                }),
            )
            .unwrap()
            .unwrap(),
    )
}

fn resources_read(cx: &mut Cx, router: &mut McpRouter, uri: &str) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(format!("read-{uri}")),
                method: "resources/read".to_owned(),
                params: Expr::Map(vec![field("uri", Expr::String(uri.to_owned()))]),
            }),
        )
        .unwrap()
        .unwrap()
}

fn native_resource(name: &str, uri: &str, capabilities: Vec<CapabilityName>) -> McpNativeCard {
    let mut card = McpNativeCard::new(
        Symbol::qualified("fixture", format!("missing-{name}")),
        "Native resource",
    )
    .with_shapes(any_shape("args"), any_shape("result"))
    .exported(
        McpExportFacet::resource()
            .with_name(name.to_owned())
            .with_uri(uri.to_owned())
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

fn native_tool(name: &str) -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("fixture", name.to_owned()), "Native tool")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(McpExportFacet::tool().with_name(name.to_owned()))
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

fn resource_uris(result: &Expr) -> Vec<String> {
    resources(result)
        .iter()
        .filter_map(|resource| field_value(resource, "uri"))
        .filter_map(|value| match value {
            Expr::String(uri) => Some(uri.clone()),
            _ => None,
        })
        .collect()
}

fn resources(result: &Expr) -> &[Expr] {
    let Some(Expr::List(resources)) = field_value(result, "resources") else {
        panic!("resources/list result should contain resources list");
    };
    resources
}

fn single_json_content(result: &Expr) -> Option<&Expr> {
    single_content(result).and_then(|content| field_value(content, "json"))
}

fn single_content(result: &Expr) -> Option<&Expr> {
    match field_value(result, "contents") {
        Some(Expr::List(items)) if items.len() == 1 => items.first(),
        _ => None,
    }
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

use sim_kernel::testing::eager_cx as cx;

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
        description: "A skill projected through resources/read.".to_owned(),
        input_shape: any_shape("skill-args"),
        output_shape: any_shape("skill-result"),
        transport_id: "resources-transport".to_owned(),
        operation: "echo".to_owned(),
    });
    card.roles = vec![role];
    card
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx, card: sim_lib_skill::SkillCard) -> Arc<sim_lib_skill::FixtureTransport> {
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("resources-transport"));
    fixture
        .insert("echo", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    registry.install_transport(fixture.clone()).unwrap();
    registry.bind_card(cx, card).unwrap();
    fixture
}
