use std::sync::Arc;

use serde_json::{Value, json};
use sim_kernel::{Args, Callable, CapabilitySet, Error};

use crate::routes::admin::admin_expr_json;
use crate::{
    ADMIN_CACHE_STATS_PATH, ADMIN_CAPABILITY_REPORT_PATH, ADMIN_EVENTS_PATH,
    ADMIN_MODEL_HEALTH_PATH, ADMIN_RUN_RETRIEVAL_PREFIX, ADMIN_RUNS_PATH, ADMIN_STORAGE_STATS_PATH,
    GatewayRequest, GatewayResponse, GatewayRouteState, GatewayRoutes, GatewayRoutesValue,
    HEALTH_PATH, OPENAI_GATEWAY_ADMIN_CAPABILITY, OpenAiGatewayFunction, OpenAiKeyTable,
    RESPONSES_PATH, cache_stats_symbol, capability_report_symbol, configure_routes,
    configure_routes_with_state, events_symbol, install_openai_gateway_lib, model_health_symbol,
    openai_gateway_admin_capability, run_get_symbol, runs_symbol, storage_stats_symbol,
};

const ADMIN_SECRET: &str = "sk-admin-route";

#[test]
fn public_health_does_not_leak_host_details() {
    let response = configure_routes().handle(&GatewayRequest::get(HEALTH_PATH));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(json["status"], "ok");
    for leaked in ["host", "hostname", "pid", "address", "uptime", "cwd"] {
        assert!(json.get(leaked).is_none(), "{leaked} leaked from health");
    }
}

#[test]
fn admin_routes_require_capability_and_return_json_metrics() {
    let routes = routes_with_admin_key();
    let response = routes.handle(&json_request(
        "POST",
        RESPONSES_PATH,
        &json!({
            "model": "fixture/slow-echo",
            "input": "admin metrics",
            "store": true,
        })
        .to_string(),
    ));
    assert_eq!(response.status(), 200);

    let denied = routes.handle(&GatewayRequest::get(ADMIN_STORAGE_STATS_PATH));
    assert_eq!(denied.status(), 403);
    assert_eq!(response_json(&denied)["error"]["code"], "capability_denied");

    let header_only = routes.handle(&header_admin_get(ADMIN_STORAGE_STATS_PATH));
    assert_eq!(header_only.status(), 403);
    assert_eq!(
        response_json(&header_only)["error"]["code"],
        "capability_denied"
    );

    let stats = response_json(&routes.handle(&admin_get(ADMIN_STORAGE_STATS_PATH)));
    assert_eq!(stats["object"], "openai-gateway/storage-stats");
    assert_eq!(stats["counters"]["request-count"], 1);
    assert_eq!(stats["counters"]["run-count"], 1);
    assert_eq!(stats["counters"]["event-count"], 6);
    assert_eq!(stats["storage-object-counts"]["response-objects"], 1);
    assert_eq!(
        stats["average-latency-ms-by-model-id"]["fixture/slow-echo"],
        1000
    );

    let runs = response_json(&routes.handle(&admin_get(ADMIN_RUNS_PATH)));
    let run_id = runs["data"][0]["id"].as_str().unwrap();
    let run =
        response_json(&routes.handle(&admin_get(&format!("{ADMIN_RUN_RETRIEVAL_PREFIX}{run_id}"))));
    assert_eq!(run["object"], "openai-gateway/run");
    assert_eq!(run["id"], run_id);

    let events = response_json(&routes.handle(&admin_get(ADMIN_EVENTS_PATH)));
    assert_eq!(events["object"], "openai-gateway/events");
    assert_eq!(events["data"].as_array().unwrap().len(), 6);
    assert!(
        events["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event-kind"] == "request-start")
    );

    let health = response_json(&routes.handle(&admin_get(ADMIN_MODEL_HEALTH_PATH)));
    assert_eq!(health["object"], "openai-gateway/model-health");
    assert_eq!(health["backend"], "sim");
    let cache = response_json(&routes.handle(&admin_get(ADMIN_CACHE_STATS_PATH)));
    assert_eq!(cache["object"], "openai-gateway/cache-stats");
    assert_eq!(cache["hit-rate"], 0.0);
    let report = response_json(&routes.handle(&admin_get(ADMIN_CAPABILITY_REPORT_PATH)));
    assert_eq!(report["object"], "openai-gateway/capability-report");
    assert_eq!(report["admin-capability"], OPENAI_GATEWAY_ADMIN_CAPABILITY);
}

#[test]
fn admin_sim_exports_require_admin_and_return_sim_values() {
    let mut cx = super::cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    for symbol in [
        runs_symbol(),
        run_get_symbol(),
        events_symbol(),
        storage_stats_symbol(),
        model_health_symbol(),
        cache_stats_symbol(),
        capability_report_symbol(),
    ] {
        assert!(cx.resolve_function(&symbol).is_ok());
    }

    let err = OpenAiGatewayFunction::storage_stats()
        .call(&mut cx, Args::new(Vec::new()))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == openai_gateway_admin_capability()
    ));

    cx.grant(openai_gateway_admin_capability());
    let routes_value = cx
        .factory()
        .opaque(Arc::new(GatewayRoutesValue::new(configure_routes())))
        .unwrap();
    let value = OpenAiGatewayFunction::storage_stats()
        .call(&mut cx, Args::new(vec![routes_value]))
        .unwrap();
    let expr = value.object().as_expr(&mut cx).unwrap();
    let json = admin_expr_json(&expr);

    assert_eq!(json["object"], "openai-gateway/storage-stats");
    assert_eq!(json["counters"]["request-count"], 0);
}

fn admin_get(path: &str) -> GatewayRequest {
    GatewayRequest::new(
        "GET",
        path,
        vec![("Authorization".to_owned(), format!("Bearer {ADMIN_SECRET}"))],
        Vec::new(),
    )
}

fn header_admin_get(path: &str) -> GatewayRequest {
    GatewayRequest::new(
        "GET",
        path,
        vec![(
            "X-SIM-Capability".to_owned(),
            OPENAI_GATEWAY_ADMIN_CAPABILITY.to_owned(),
        )],
        Vec::new(),
    )
}

fn routes_with_admin_key() -> GatewayRoutes {
    let key_table = OpenAiKeyTable::new().unwrap();
    key_table
        .add_secret(
            ADMIN_SECRET,
            CapabilitySet::new().grant(openai_gateway_admin_capability()),
        )
        .unwrap();
    configure_routes_with_state(GatewayRouteState::memory().with_keys(key_table))
}

fn json_request(method: &str, path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        method,
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}
