use std::sync::Arc;

use sim_codec_mcp::{
    EXECUTION_ERROR, METHOD_NOT_FOUND, McpEnvelope, McpErrorEnvelope, McpNotification, McpRequest,
    McpResponse, envelope_to_expr, expr_to_envelope,
};
#[cfg(feature = "skill")]
use sim_kernel::Cx;
use sim_kernel::{Args, CapabilityName, Expr, ShapeRef, Symbol};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpAnnotation, McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSurfaceRole,
    McpSurfaceSource, NativeFacet, handle_symbol, health_symbol, initialize_symbol,
    install_mcp_lib, mcp_export_facet_name, mcp_export_operation_symbol, project_native_surface,
};

#[test]
fn surface_native_cards_without_mcp_export_are_invisible() {
    let card = McpNativeCard::new(Symbol::qualified("core", "reverse"), "Reverse values")
        .with_facet(NativeFacet::Other {
            name: Symbol::new("other"),
            value: Expr::Nil,
            private: false,
        });

    let rows = project_native_surface(&[card], &McpProfile::all()).unwrap();

    assert!(rows.is_empty());
}

#[test]
fn surface_native_mcp_export_facet_and_operation_are_named() {
    assert_eq!(mcp_export_facet_name(), Symbol::new("mcp-export"));
    assert_eq!(
        mcp_export_operation_symbol(),
        Symbol::qualified("mcp", "export")
    );
}

#[test]
fn surface_native_exports_use_stable_names_and_profiles() {
    let exported = native_tool(Symbol::qualified("core", "reverse"), None);
    let hidden = native_tool(
        Symbol::qualified("core", "hidden"),
        Some("core.hidden".to_owned()),
    );
    let profile = McpProfile::all().with_denied_name("core.hidden");

    let rows = project_native_surface(&[exported, hidden], &profile).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "core.reverse");
    assert_eq!(rows[0].source, McpSurfaceSource::NativeCard);
    assert_eq!(rows[0].role, McpSurfaceRole::Tool);
}

#[test]
fn surface_collisions_fail_at_projection_time() {
    let first = native_tool(
        Symbol::qualified("core", "one"),
        Some("duplicate.name".to_owned()),
    );
    let second = native_tool(
        Symbol::qualified("core", "two"),
        Some("duplicate.name".to_owned()),
    );

    let err = match project_native_surface(&[first, second], &McpProfile::all()) {
        Ok(_) => panic!("duplicate names should fail during projection"),
        Err(err) => err,
    };

    assert!(format!("{err}").contains("collision"));
}

#[test]
fn surface_native_projection_redacts_private_annotations() {
    let card = McpNativeCard::new(Symbol::qualified("core", "redacted"), "Redacted").exported(
        McpExportFacet::tool()
            .with_name("core.redacted")
            .with_annotation(McpAnnotation::public(
                Symbol::new("safe"),
                Expr::String("public".to_owned()),
            ))
            .with_annotation(McpAnnotation::private(
                Symbol::new("secret"),
                Expr::String("private-token".to_owned()),
            )),
    );

    let rows = project_native_surface(&[card], &McpProfile::all()).unwrap();

    assert_eq!(rows[0].annotations.len(), 1);
    assert_eq!(rows[0].annotations[0].0, Symbol::new("safe"));
    assert_eq!(rows[0].annotations[0].1, Expr::String("public".to_owned()));
}

#[test]
fn surface_rows_do_not_store_native_private_state() {
    let mut cx = cx();
    let card = McpNativeCard::new(Symbol::qualified("core", "safe"), "Safe").exported(
        McpExportFacet::tool().with_annotation(McpAnnotation::private(
            Symbol::new("transport"),
            Expr::String("secret-transport".to_owned()),
        )),
    );

    let rows = project_native_surface(&[card], &McpProfile::all()).unwrap();
    let expr = rows[0].to_expr(&mut cx).unwrap();

    assert!(!format!("{expr:?}").contains("secret-transport"));
}

#[test]
fn router_request_ids_are_preserved() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();
    let id = Expr::String("req-1".to_owned());

    let reply = router
        .handle(&mut cx, request(id.clone(), "ping", Expr::Nil))
        .unwrap()
        .unwrap();

    let McpEnvelope::Response(McpResponse {
        id: response_id, ..
    }) = reply
    else {
        panic!("ping should return a response");
    };
    assert_eq!(response_id, id);
}

#[test]
fn router_notifications_return_no_response() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();

    let reply = router
        .handle(
            &mut cx,
            McpEnvelope::Notification(McpNotification {
                method: "notifications/initialized".to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap();

    assert!(reply.is_none());
    assert!(router.session().initialized);
}

#[test]
fn router_ping_works_before_initialization() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();

    let reply = router
        .handle(
            &mut cx,
            request(Expr::String("p".to_owned()), "ping", Expr::Nil),
        )
        .unwrap()
        .unwrap();

    assert!(!router.session().initialized);
    assert!(matches!(reply, McpEnvelope::Response(_)));
}

#[test]
fn router_unknown_methods_return_method_not_found() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();
    let id = Expr::String("bad-method".to_owned());

    let reply = router
        .handle(
            &mut cx,
            request(id.clone(), "does/not-exist", Expr::Map(Vec::new())),
        )
        .unwrap()
        .unwrap();

    let error = expect_error(reply);
    assert_eq!(error.id, id);
    assert_eq!(error.error.code, METHOD_NOT_FOUND);
}

#[test]
fn router_tools_call_unknown_tool_returns_json_rpc_error() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();

    let reply = router
        .handle(
            &mut cx,
            request(
                Expr::String("tool-call".to_owned()),
                "tools/call",
                Expr::Map(vec![
                    field("name", Expr::String("missing.tool".to_owned())),
                    field("arguments", Expr::List(Vec::new())),
                ]),
            ),
        )
        .unwrap()
        .unwrap();

    assert_eq!(expect_error(reply).error.code, EXECUTION_ERROR);
}

#[test]
fn router_handle_function_returns_response_expr_and_nil_for_notifications() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();
    let ping = cx
        .factory()
        .expr(envelope_to_expr(&request(
            Expr::String("call-1".to_owned()),
            "ping",
            Expr::Nil,
        )))
        .unwrap();

    let response = cx
        .call_function(&handle_symbol(), Args::new(vec![ping]))
        .unwrap();
    let response = expr_to_envelope(&response.object().as_expr(&mut cx).unwrap()).unwrap();
    assert!(matches!(
        response,
        McpEnvelope::Response(McpResponse { id: Expr::String(id), .. }) if id == "call-1"
    ));

    let notification = cx
        .factory()
        .expr(envelope_to_expr(&McpEnvelope::Notification(
            McpNotification {
                method: "notifications/initialized".to_owned(),
                params: Expr::Nil,
            },
        )))
        .unwrap();
    let nil = cx
        .call_function(&handle_symbol(), Args::new(vec![notification]))
        .unwrap();
    assert_eq!(nil.object().as_expr(&mut cx).unwrap(), Expr::Nil);
}

#[test]
fn router_initialize_and_health_functions_return_public_data() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();

    let initialize = cx
        .call_function(&initialize_symbol(), Args::default())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(map_field(&initialize, "serverInfo").is_some());
    assert!(map_field(&initialize, "capabilities").is_some());

    let health = cx
        .call_function(&health_symbol(), Args::default())
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(map_field(&health, "initialized"), Some(&Expr::Bool(false)));
    assert!(map_field(&health, "activeRequests").is_some());
}

#[cfg(feature = "skill")]
#[test]
fn surface_skill_cards_without_mcp_role_are_invisible() {
    use sim_lib_skill::SkillRole;

    let mut cx = skill_cx();
    let mut card = skill_card("memory.only");
    card.roles = vec![SkillRole::Memory];
    bind_skill(&mut cx, card);

    let rows = crate::project_skill_surface(&mut cx, &McpProfile::all()).unwrap();

    assert!(rows.is_empty());
}

#[cfg(feature = "skill")]
#[test]
fn surface_profiles_filter_native_and_skill_rows_identically() {
    let mut cx = skill_cx();
    bind_skill(&mut cx, skill_card("skill.visible"));
    let native = native_tool(
        Symbol::qualified("core", "visible"),
        Some("native.visible".to_owned()),
    );

    let native_only = crate::project_mcp_surface(
        &mut cx,
        std::slice::from_ref(&native),
        &McpProfile::allow_names(["native.visible"]),
    )
    .unwrap();
    let skill_only = crate::project_mcp_surface(
        &mut cx,
        &[native],
        &McpProfile::allow_names(["skill.visible"]),
    )
    .unwrap();

    assert_eq!(names(&native_only), vec!["native.visible"]);
    assert_eq!(names(&skill_only), vec!["skill.visible"]);
}

#[cfg(feature = "skill")]
#[test]
fn surface_skill_rows_point_at_bound_skill_callable_symbol() {
    let mut cx = skill_cx();
    bind_skill(&mut cx, skill_card("fixture.echo"));

    let rows = crate::project_skill_surface(&mut cx, &McpProfile::all()).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "fixture.echo");
    assert_eq!(rows[0].source, McpSurfaceSource::SkillCard);
    assert_eq!(
        rows[0].symbol,
        Some(Symbol::qualified("skill", "fixture.echo"))
    );
}

#[cfg(feature = "skill")]
#[test]
fn surface_native_and_skill_name_collisions_fail_before_call_time() {
    let mut cx = skill_cx();
    bind_skill(&mut cx, skill_card("same.name"));
    let native = native_tool(
        Symbol::qualified("core", "same"),
        Some("same.name".to_owned()),
    );

    let err = match crate::project_mcp_surface(&mut cx, &[native], &McpProfile::all()) {
        Ok(_) => panic!("duplicate names should fail during projection"),
        Err(err) => err,
    };

    assert!(format!("{err}").contains("collision"));
}

#[cfg(feature = "skill")]
#[test]
fn surface_skill_projection_omits_transport_state() {
    let mut cx = skill_cx();
    bind_skill(&mut cx, skill_card("secret.skill"));

    let rows = crate::project_skill_surface(&mut cx, &McpProfile::all()).unwrap();
    let expr = rows[0].to_expr(&mut cx).unwrap();
    let text = format!("{expr:?}");

    assert!(!text.contains("secret-transport"));
    assert!(!text.contains("private-op"));
}

fn native_tool(symbol: Symbol, name: Option<String>) -> McpNativeCard {
    let mut export = McpExportFacet::tool();
    if let Some(name) = name {
        export = export.with_name(name);
    }
    McpNativeCard::new(symbol, "Native tool")
        .with_shapes(any_shape("args"), any_shape("result"))
        .with_capability(CapabilityName::new("native.call"))
        .exported(export)
}

use sim_kernel::testing::eager_cx as cx;

fn request(id: Expr, method: &str, params: Expr) -> McpEnvelope {
    McpEnvelope::Request(McpRequest {
        id,
        method: method.to_owned(),
        params,
    })
}

fn expect_error(envelope: McpEnvelope) -> McpErrorEnvelope {
    let McpEnvelope::Error(error) = envelope else {
        panic!("expected MCP error envelope");
    };
    error
}

use sim_value::access::field_any as map_field;

use sim_value::build::entry as field;

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
        description: "A skill projected through MCP surface rows.".to_owned(),
        input_shape: any_shape("skill-args"),
        output_shape: any_shape("skill-result"),
        transport_id: "secret-transport".to_owned(),
        operation: "private-op".to_owned(),
    })
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx, card: sim_lib_skill::SkillCard) {
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("secret-transport"));
    fixture
        .insert("private-op", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    registry.install_transport(fixture).unwrap();
    registry.bind_card(cx, card).unwrap();
}

#[cfg(feature = "skill")]
fn names(rows: &[crate::McpSurfaceCard]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}
