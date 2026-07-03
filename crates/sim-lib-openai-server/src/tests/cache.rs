use serde_json::Value;
use sim_kernel::Expr;

use crate::{
    DeterministicGatewayClock, GatewayEvent, GatewayRequest, GatewayStore, MemoryGatewayStore,
    OpenAiPlanCache, PlanCacheKey, PlanCacheWriteTarget, RESPONSES_PATH, ResponseIdGenerators,
    ai_runner_cache_capability, execute_response_request_with_cache,
    openai_gateway_plan_capability,
};

#[test]
fn repeated_cache_plan_returns_hit_without_backend_branch() {
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    let mut store = MemoryGatewayStore::new();
    let mut cache = OpenAiPlanCache::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicGatewayClock::new(1_000, 10);
    let request =
        responses_request(r#"{"model":"cache(fixture/echo)","input":"cache me","store":true}"#);

    let first = execute_response_request_with_cache(
        &mut cx, &mut store, &mut cache, &mut ids, &mut clock, &request,
    );
    let second = execute_response_request_with_cache(
        &mut cx, &mut store, &mut cache, &mut ids, &mut clock, &request,
    );

    assert_eq!(first.response().status(), 200);
    assert_eq!(second.response().status(), 200);
    assert_eq!(response_json(second.response())["output_text"], "cache me");
    assert_eq!(cache.len(), 1);
    assert!(event_kind(first.events(), "branch-start"));
    assert!(!event_kind(first.events(), "cache-hit"));
    assert!(event_kind(second.events(), "cache-hit"));
    assert!(!event_kind(second.events(), "branch-start"));
    assert!(
        stored_events(&store, second.event_content_ids())
            .iter()
            .any(|event| event.kind().name.as_ref() == "cache-hit")
    );
}

#[test]
fn refresh_cache_mode_bypasses_hit_and_rewrites_entry() {
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    let mut store = MemoryGatewayStore::new();
    let mut cache = OpenAiPlanCache::new();
    let mut ids = ResponseIdGenerators::deterministic(20);
    let mut clock = DeterministicGatewayClock::new(2_000, 10);
    let read_through =
        responses_request(r#"{"model":"cache(fixture/echo)","input":"refresh me","store":true}"#);
    let refresh = responses_request(
        r#"{"model":"cache(fixture/echo, mode: refresh)","input":"refresh me","store":true}"#,
    );

    let first = execute_response_request_with_cache(
        &mut cx,
        &mut store,
        &mut cache,
        &mut ids,
        &mut clock,
        &read_through,
    );
    let refreshed = execute_response_request_with_cache(
        &mut cx, &mut store, &mut cache, &mut ids, &mut clock, &refresh,
    );
    let hit = execute_response_request_with_cache(
        &mut cx,
        &mut store,
        &mut cache,
        &mut ids,
        &mut clock,
        &read_through,
    );

    assert_eq!(first.response().status(), 200);
    assert_eq!(refreshed.response().status(), 200);
    assert!(event_kind(refreshed.events(), "branch-start"));
    assert!(!event_kind(refreshed.events(), "cache-hit"));
    assert!(event_kind(hit.events(), "cache-hit"));
}

#[test]
fn persistent_plan_cache_writes_require_ai_runner_cache() {
    let mut cx = super::cx();
    let key = PlanCacheKey::for_request_plan(
        &Expr::String("request".to_owned()),
        &Expr::String("plan".to_owned()),
    )
    .unwrap();
    let response = Expr::String("cached response".to_owned());
    let mut cache = OpenAiPlanCache::new();

    let err = cache
        .put(
            &mut cx,
            PlanCacheWriteTarget::Persistent,
            key.clone(),
            response.clone(),
        )
        .unwrap_err();
    assert!(
        matches!(err, sim_kernel::Error::CapabilityDenied { capability } if capability == ai_runner_cache_capability())
    );

    cx.grant(ai_runner_cache_capability());
    cache
        .put(
            &mut cx,
            PlanCacheWriteTarget::Persistent,
            key.clone(),
            response.clone(),
        )
        .unwrap();
    assert_eq!(cache.get(&key), Some(&response));
}

fn responses_request(body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &crate::GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn stored_events(store: &MemoryGatewayStore, ids: &[sim_kernel::ContentId]) -> Vec<GatewayEvent> {
    ids.iter().map(|id| store.event(id).unwrap()).collect()
}

fn event_kind(events: &[GatewayEvent], kind: &str) -> bool {
    events
        .iter()
        .any(|event| event.kind().name.as_ref() == kind)
}
