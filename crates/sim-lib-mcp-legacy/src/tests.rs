use super::*;
use sim_kernel::testing::bare_cx;
use sim_lib_mcp::{McpProfile, ServerDescription};

fn request(id: &str, method: &str, params: Expr) -> McpRequest {
    McpRequest {
        id: Expr::String(id.into()),
        method: method.into(),
        params,
    }
}

#[test]
fn delivered_2025_03_26_lifecycle_vector_is_preserved() {
    let service = McpService::new(ServerDescription::new(
        "sim",
        env!("CARGO_PKG_VERSION"),
        McpProfile::all(),
    ));
    let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
    let mut cx = bare_cx();
    let replies = adapter
        .handle(&mut cx, request("1", "initialize", Expr::Nil))
        .unwrap();
    assert!(matches!(&replies[0], McpEnvelope::Response(_)));
    adapter
        .handle(
            &mut cx,
            request("2", "notifications/initialized", Expr::Nil),
        )
        .unwrap();
    assert!(adapter.initialized());
    adapter
        .handle(&mut cx, request("3", "ping", Expr::Nil))
        .unwrap();
    adapter
        .handle(&mut cx, request("4", "shutdown", Expr::Nil))
        .unwrap();
    assert!(adapter.shutdown_requested());
}

#[test]
fn delivered_2025_03_26_stateless_vectors_use_the_identical_service_path() {
    for method in ["ping", "resources/list", "prompts/list", "tools/list"] {
        let direct = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let context = RequestContext::new(
            "direct",
            "2025-03-26",
            NegotiatedExtensions::none(),
            Principal::new("client"),
            CachePolicy::Bypass,
        );
        let expected: Vec<_> = direct
            .handle(
                &mut bare_cx(),
                &context,
                request("vector", method, Expr::Nil),
            )
            .unwrap()
            .collect();

        let adapted = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let mut connection = LegacyConnection::new(adapted, "legacy", Principal::new("client"));
        let actual = connection
            .handle(&mut bare_cx(), request("vector", method, Expr::Nil))
            .unwrap();
        assert_eq!(actual, expected, "legacy vector diverged for {method}");
    }
}

#[test]
fn initialized_notification_is_connection_state_and_has_no_reply() {
    let service = McpService::new(ServerDescription::new(
        "sim",
        env!("CARGO_PKG_VERSION"),
        McpProfile::all(),
    ));
    let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
    let replies = adapter
        .handle_envelope(
            &mut bare_cx(),
            McpEnvelope::Notification(McpNotification {
                method: "notifications/initialized".to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap();
    assert!(replies.is_empty());
    assert!(adapter.initialized());
}

#[test]
fn invalid_initialize_params_keep_the_delivered_type_error() {
    let service = McpService::new(ServerDescription::new(
        "sim",
        env!("CARGO_PKG_VERSION"),
        McpProfile::all(),
    ));
    let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
    assert!(matches!(
        adapter.handle(&mut bare_cx(), request("1", "initialize", Expr::Bool(true))),
        Err(Error::TypeMismatch { .. })
    ));
}

#[test]
fn initialize_retains_explicit_version_extension_and_grant_facts() {
    let service = McpService::new(ServerDescription::new(
        "sim",
        env!("CARGO_PKG_VERSION"),
        McpProfile::all(),
    ));
    let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
    let field = |name: &str, value| (Expr::Symbol(Symbol::new(name)), value);
    let params = Expr::Map(vec![
        field("protocolVersion", Expr::String("2025-03-26".to_owned())),
        (
            Expr::Symbol(Symbol::new("capabilities")),
            Expr::Map(vec![field("sampling", Expr::Map(Vec::new()))]),
        ),
        field(
            "grants",
            Expr::Vector(vec![Expr::String("tools.call".to_owned())]),
        ),
    ]);
    adapter
        .handle(&mut bare_cx(), request("1", "initialize", params))
        .unwrap();
    assert_eq!(adapter.protocol_version(), "2025-03-26");
    assert!(adapter.negotiated_extensions().contains("sampling"));
    assert_eq!(adapter.requested_grants(), ["tools.call"]);
}
