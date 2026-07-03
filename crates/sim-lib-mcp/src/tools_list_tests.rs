use std::sync::Arc;

use sim_codec_mcp::{McpEnvelope, McpRequest, McpResponse};
use sim_kernel::{Args, CapabilityName, Cx, Expr, ShapeRef, Symbol};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpAnnotation, McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession,
    McpSurfaceRole, install_mcp_lib, tools_symbol,
};

#[test]
fn tools_list_only_allowed_native_tool_rows_appear() {
    let mut cx = cx();
    let session = McpSession::new("tools", McpProfile::all().with_denied_name("denied.tool"))
        .with_native_cards(vec![
            native_tool("allowed.tool", Vec::new()),
            native_tool("denied.tool", Vec::new()),
            native_resource("visible.resource"),
        ]);
    let mut router = McpRouter::new(session);

    let result = tools_list_result(&mut cx, &mut router);

    assert_eq!(tool_names(&result), vec!["allowed.tool"]);
}

#[test]
fn tools_list_filters_missing_capabilities() {
    let mut cx = cx();
    let allowed = CapabilityName::new("mcp.allowed");
    let session = McpSession::new("tools", McpProfile::all())
        .with_granted_capability(allowed.clone())
        .with_native_cards(vec![
            native_tool("allowed.tool", vec![allowed]),
            native_tool("blocked.tool", vec![CapabilityName::new("mcp.blocked")]),
        ]);
    let mut router = McpRouter::new(session);

    let result = tools_list_result(&mut cx, &mut router);

    assert_eq!(tool_names(&result), vec!["allowed.tool"]);
}

#[test]
fn tools_list_input_schema_is_deterministic() {
    let mut cx = cx();
    let session = McpSession::new("tools", McpProfile::all())
        .with_native_cards(vec![native_tool("schema.tool", Vec::new())]);
    let mut first_router = McpRouter::new(session.clone());
    let mut second_router = McpRouter::new(session);

    let first = tools_list_result(&mut cx, &mut first_router);
    let second = tools_list_result(&mut cx, &mut second_router);

    assert_eq!(
        tool_schema(&first, "schema.tool"),
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("x-sim-shape")),
            Expr::String("mcp-test/args".to_owned()),
        )])
    );
    assert_eq!(first, second);
}

#[test]
fn tools_list_descriptors_expose_no_secrets_or_private_capabilities() {
    let mut cx = cx();
    let secret_capability = CapabilityName::new("secret.capability");
    let session = McpSession::new("tools", McpProfile::all())
        .with_granted_capability(secret_capability.clone())
        .with_native_cards(vec![secret_tool("secret.tool", secret_capability)]);
    let mut router = McpRouter::new(session);

    let result = tools_list_result(&mut cx, &mut router);
    let text = format!("{result:?}");

    assert_eq!(tool_names(&result), vec!["secret.tool"]);
    assert!(!text.contains("secret.capability"));
    assert!(!text.contains("private-token"));
}

#[test]
fn tools_list_mcp_tools_function_returns_empty_fixture_list() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();

    let result = cx
        .call_function(&tools_symbol(), Args::default())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert_eq!(tool_names(&result), Vec::<String>::new());
}

#[cfg(feature = "skill")]
#[test]
fn tools_list_skill_rows_use_same_profile_filter() {
    let mut cx = skill_cx();
    bind_skill(&mut cx, skill_card("skill.visible"));
    bind_skill(&mut cx, skill_card("skill.hidden"));
    let session = McpSession::new("skills", McpProfile::all().with_denied_name("skill.hidden"))
        .with_granted_capability(sim_lib_skill::skill_specific_call_capability(
            "skill.visible",
        ));
    let mut router = McpRouter::new(session);

    let result = tools_list_result(&mut cx, &mut router);

    assert_eq!(tool_names(&result), vec!["skill.visible"]);
}

fn tools_list_result(cx: &mut Cx, router: &mut McpRouter) -> Expr {
    let response = router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String("tools".to_owned()),
                method: "tools/list".to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap()
        .unwrap();
    let McpEnvelope::Response(McpResponse { result, .. }) = response else {
        panic!("tools/list should return a response");
    };
    result
}

fn native_tool(name: &str, capabilities: Vec<CapabilityName>) -> McpNativeCard {
    let mut card = McpNativeCard::new(Symbol::qualified("native", name.to_owned()), "Native tool")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(McpExportFacet::tool().with_name(name.to_owned()));
    for capability in capabilities {
        card = card.with_capability(capability);
    }
    card
}

fn native_resource(name: &str) -> McpNativeCard {
    McpNativeCard::new(
        Symbol::qualified("native", name.to_owned()),
        "Native resource",
    )
    .with_shapes(any_shape("args"), any_shape("result"))
    .exported(McpExportFacet::new(McpSurfaceRole::Resource).with_name(name.to_owned()))
}

fn secret_tool(name: &str, capability: CapabilityName) -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("native", name.to_owned()), "Secret tool")
        .with_shapes(any_shape("args"), any_shape("result"))
        .with_capability(capability)
        .exported(
            McpExportFacet::tool()
                .with_name(name.to_owned())
                .with_annotation(McpAnnotation::private(
                    Symbol::new("secret"),
                    Expr::String("private-token".to_owned()),
                )),
        )
}

fn tool_names(result: &Expr) -> Vec<String> {
    tools(result)
        .iter()
        .filter_map(|tool| field_value(tool, "name"))
        .filter_map(|value| match value {
            Expr::String(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn tool_schema(result: &Expr, name: &str) -> Expr {
    tools(result)
        .iter()
        .find(
            |tool| matches!(field_value(tool, "name"), Some(Expr::String(found)) if found == name),
        )
        .and_then(|tool| field_value(tool, "inputSchema"))
        .cloned()
        .unwrap_or(Expr::Nil)
}

fn tools(result: &Expr) -> &[Expr] {
    let Some(Expr::List(tools)) = field_value(result, "tools") else {
        panic!("tools/list result should contain tools list");
    };
    tools
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

#[cfg(feature = "skill")]
fn skill_cx() -> Cx {
    let mut cx = cx();
    sim_lib_skill::install_skill_lib(&mut cx).unwrap();
    cx
}

#[cfg(feature = "skill")]
fn skill_card(id: &str) -> sim_lib_skill::SkillCard {
    sim_lib_skill::SkillCard::fixture(sim_lib_skill::FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Fixture Skill".to_owned(),
        description: "A skill projected through tools/list.".to_owned(),
        input_shape: any_shape("skill-args"),
        output_shape: any_shape("skill-result"),
        transport_id: "tools-list-transport".to_owned(),
        operation: "echo".to_owned(),
    })
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx, card: sim_lib_skill::SkillCard) {
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("tools-list-transport"));
    fixture
        .insert("echo", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    registry.install_transport(fixture).unwrap();
    registry.bind_card(cx, card).unwrap();
}
