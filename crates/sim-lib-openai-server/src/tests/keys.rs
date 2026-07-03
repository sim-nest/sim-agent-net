use sim_kernel::{Args, Callable, CapabilitySet, Error};

use crate::{
    GatewayRequest, GatewayResponseObjectStore, GatewayRouteState, GatewayStore,
    OPENAI_GATEWAY_ADMIN_CAPABILITY, OPENAI_GATEWAY_PLAN_CAPABILITY, OpenAiGatewayFunction,
    OpenAiKeyTable, RESPONSES_PATH, configure_routes, configure_routes_with_state,
    install_openai_gateway_lib, key_add_symbol, key_list_symbol, openai_gateway_admin_capability,
    openai_gateway_plan_capability,
};

#[test]
fn api_key_with_only_local_capabilities_cannot_run_remote_plan() {
    let key_table = OpenAiKeyTable::new().unwrap();
    key_table
        .add_secret(
            "sk-local-only",
            CapabilitySet::new().grant(openai_gateway_plan_capability()),
        )
        .unwrap();
    let routes = configure_routes_with_state(GatewayRouteState::memory().with_keys(key_table));

    let response = routes.handle(&authorized_responses_request(
        "sk-local-only",
        r#"{"model":"remote(fixture/echo)","input":"remote blocked","store":false}"#,
    ));
    let body = std::str::from_utf8(response.body()).unwrap();

    assert_ne!(response.status(), 200);
    assert!(body.contains("openai-gateway.plan.remote"));
}

#[test]
fn unknown_api_key_gets_anonymous_capabilities_and_fails_closed() {
    let routes = configure_routes_with_state(GatewayRouteState::memory());

    let response = routes.handle(&authorized_responses_request(
        "sk-unknown",
        r#"{"model":"race(fixture/echo, fixture/slow-echo)","input":"blocked","store":false}"#,
    ));
    let body = std::str::from_utf8(response.body()).unwrap();

    assert_ne!(response.status(), 200);
    assert!(body.contains("openai-gateway.plan"));
}

#[test]
fn api_key_value_is_absent_from_stored_request_and_sim_trace() {
    let secret = "sk-redaction-secret";
    let key_table = OpenAiKeyTable::new().unwrap();
    key_table.add_secret(secret, CapabilitySet::new()).unwrap();
    let routes = configure_routes_with_state(GatewayRouteState::memory().with_keys(key_table));

    let stored = routes.handle(&authorized_responses_request(
        secret,
        r#"{"model":"fixture/echo","input":"redact me","store":true}"#,
    ));
    let response_id = response_json(&stored)["id"].as_str().unwrap().to_owned();
    let sim = routes.handle(&GatewayRequest::new(
        "GET",
        format!("{RESPONSES_PATH}/{response_id}/sim"),
        vec![(
            "X-SIM-Capability".to_owned(),
            OPENAI_GATEWAY_ADMIN_CAPABILITY.to_owned(),
        )],
        Vec::new(),
    ));
    let sim_body = std::str::from_utf8(sim.body()).unwrap();

    assert_eq!(stored.status(), 200);
    assert_eq!(sim.status(), 200);
    assert!(!sim_body.contains(secret));
    let store = routes.state().store().lock().unwrap();
    let record = store.response_object(&response_id).unwrap();
    let request = store
        .request(record.request_content_id.as_ref().unwrap())
        .unwrap();
    assert!(!format!("{:?}", request.to_expr()).contains(secret));
}

#[test]
fn key_admin_functions_are_installed_and_never_return_secret_values() {
    let mut cx = super::cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    assert!(cx.resolve_function(&key_add_symbol()).is_ok());
    assert!(cx.resolve_function(&key_list_symbol()).is_ok());

    let secret = "sk-admin-redaction-secret";
    let secret_value = cx.factory().string(secret.to_owned()).unwrap();
    let err = OpenAiGatewayFunction::key_add()
        .call(&mut cx, Args::new(vec![secret_value]))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == openai_gateway_admin_capability()
    ));

    cx.grant(openai_gateway_admin_capability());
    let secret_value = cx.factory().string(secret.to_owned()).unwrap();
    let capability = cx
        .factory()
        .string(OPENAI_GATEWAY_PLAN_CAPABILITY.to_owned())
        .unwrap();
    let key = OpenAiGatewayFunction::key_add()
        .call(&mut cx, Args::new(vec![secret_value, capability]))
        .unwrap();
    let key_expr = key.object().as_expr(&mut cx).unwrap();
    let key_text = format!("{key_expr:?}");
    assert!(key_text.contains("key-hash"));
    assert!(!key_text.contains(secret));

    let listed = OpenAiGatewayFunction::key_list()
        .call(&mut cx, Args::new(Vec::new()))
        .unwrap();
    let listed_text = format!("{:?}", listed.object().as_expr(&mut cx).unwrap());
    assert!(!listed_text.contains(secret));

    let response = configure_routes().handle(&authorized_responses_request(
        secret,
        r#"{"model":"race(fixture/echo, fixture/slow-echo)","input":"admin key","store":false}"#,
    ));
    assert_eq!(response.status(), 200);
}

fn authorized_responses_request(secret: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), format!("Bearer {secret}")),
        ],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &crate::GatewayResponse) -> serde_json::Value {
    serde_json::from_slice(response.body()).unwrap()
}
