use super::*;
use sim_codec_mcp::{McpEnvelope, McpRequest};
use sim_kernel::Expr;

#[test]
fn service_is_stateless_and_rejects_connection_lifecycle() {
    let service = McpService::new(ServerDescription::new("sim", "1", McpProfile::all()));
    let context = RequestContext::new(
        "r1",
        "2025-03-26",
        NegotiatedExtensions::none(),
        Principal::new("fixture"),
        CachePolicy::Bypass,
    );
    let request = McpRequest {
        id: Expr::String("1".into()),
        method: "initialize".into(),
        params: Expr::Nil,
    };
    assert!(matches!(
        service
            .handle(&mut sim_kernel::testing::bare_cx(), &context, request)
            .unwrap()
            .next(),
        Some(McpEnvelope::Error(_))
    ));
}

#[test]
fn request_factory_intersects_authority_and_discards_request_state() {
    let allowed = CapabilityName::new("mcp.allowed");
    let denied = CapabilityName::new("mcp.denied");
    let context = RequestContext::new(
        "r1",
        "2026-07-28",
        NegotiatedExtensions::none(),
        Principal::new("alice"),
        CachePolicy::Bypass,
    )
    .with_principal_grants(
        CapabilitySet::new()
            .grant(allowed.clone())
            .grant(denied.clone()),
    )
    .with_admitted_needs(CapabilitySet::new().grant(allowed.clone()));
    let mut seed = sim_kernel::testing::bare_cx();
    seed.push_info("host diagnostic must not leak");
    let factory = RequestCxFactory::new(100);
    let first = factory
        .run(&seed, &context, |cx| {
            assert!(cx.capabilities().contains(&allowed));
            assert!(!cx.capabilities().contains(&denied));
            assert!(cx.diagnostics().messages().is_empty());
            cx.push_info("request-private");
            Ok(cx.fresh_handle())
        })
        .unwrap();
    let second = factory
        .run(&seed, &context, |cx| {
            assert!(cx.diagnostics().messages().is_empty());
            Ok(cx.fresh_handle())
        })
        .unwrap();
    assert_ne!(first, second);
}

#[test]
fn cache_hints_follow_codec_registry_and_complete_status() {
    let service = McpService::new(ServerDescription::new("sim", "1", McpProfile::all()));
    let context = RequestContext::new(
        "r",
        "2026-07-28",
        NegotiatedExtensions::none(),
        Principal::new("alice"),
        CachePolicy::ReadWrite,
    );
    assert!(
        service
            .cache_hint(Method::ToolsList, &ResultType::Complete, &context)
            .is_some()
    );
    assert!(
        service
            .cache_hint(Method::ToolsCall, &ResultType::Complete, &context)
            .is_none()
    );
    assert!(
        service
            .cache_hint(Method::ToolsList, &ResultType::InputRequired, &context)
            .is_none()
    );
}

#[test]
fn randomized_request_order_cannot_change_authority_or_private_state() {
    let factory = RequestCxFactory::new(500);
    let seed = sim_kernel::testing::bare_cx();
    let read = CapabilityName::new("mcp.read");
    let write = CapabilityName::new("mcp.write");
    let contexts = [
        RequestContext::new(
            "a",
            "2026-07-28",
            NegotiatedExtensions::none(),
            Principal::new("alice"),
            CachePolicy::ReadWrite,
        )
        .with_principal_grants(CapabilitySet::new().grant(read.clone()))
        .with_admitted_needs(
            CapabilitySet::new()
                .grant(read.clone())
                .grant(write.clone()),
        ),
        RequestContext::new(
            "b",
            "2025-03-26",
            NegotiatedExtensions::none().with("example/x"),
            Principal::new("bob"),
            CachePolicy::Bypass,
        )
        .with_principal_grants(CapabilitySet::new().grant(write.clone()))
        .with_admitted_needs(CapabilitySet::new().grant(write.clone())),
    ];
    let observe = |context: &RequestContext| {
        factory
            .run(&seed, context, |cx| {
                assert!(cx.diagnostics().messages().is_empty());
                Ok(cx
                    .capabilities()
                    .iter()
                    .map(|cap| cap.as_str().to_owned())
                    .collect::<Vec<_>>())
            })
            .unwrap()
    };
    let expected = [observe(&contexts[0]), observe(&contexts[1])];
    let mut state = 0x5eed_u64;
    for _ in 0..128 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let order = if state & 1 == 0 { [0, 1] } else { [1, 0] };
        for index in order {
            assert_eq!(observe(&contexts[index]), expected[index]);
        }
    }
}
