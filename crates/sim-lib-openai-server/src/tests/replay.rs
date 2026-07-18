use serde_json::{Value, json};
use sim_kernel::CapabilitySet;

use crate::{
    GatewayRequest, GatewayResponse, GatewayRouteState, GatewayRoutes, OpenAiKeyTable,
    RESPONSES_PATH, SIM_EXTENSION_CAPABILITY, SIM_FORK_PATH, SIM_INSPECTION_CAPABILITY,
    SIM_REPLAY_PATH, configure_routes, configure_routes_with_state,
    openai_gateway_admin_capability,
};

const ADMIN_SECRET: &str = "sk-replay-admin";
const EXTENSION_SECRET: &str = "sk-replay-extension";
const INSPECT_SECRET: &str = "sk-replay-inspect";
const OTHER_EXTENSION_SECRET: &str = "sk-replay-other";

#[test]
fn replay_missing_response_id_uses_shared_missing_required_error() {
    let response = configure_routes().handle(&json_request("POST", SIM_REPLAY_PATH, "{}"));
    assert_eq!(response.status(), 400);
    let error = response_json(&response);
    assert_eq!(error["error"]["code"], "missing_required_parameter");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("response_id")
    );
}

#[test]
fn response_events_and_replay_return_stored_event_ids_without_rerun() {
    let routes = routes_with_sim_keys();
    let response_id = stored_response_id(&routes, "replay me");

    let events = routes.handle(&admin_get(&format!(
        "{RESPONSES_PATH}/{response_id}/events"
    )));
    let replay = routes.handle(&authorized_json_request(
        ADMIN_SECRET,
        "POST",
        SIM_REPLAY_PATH,
        &json!({ "response_id": response_id }).to_string(),
    ));
    let events_json = response_json(&events);
    let replay_json = response_json(&replay);

    assert_eq!(events.status(), 200);
    assert_eq!(replay.status(), 200);
    assert_eq!(events_json["data"], replay_json["data"]);
    assert_eq!(event_ids(&events_json).len(), 6);
    assert_eq!(events_json["data"][0]["event"], "request-start");
    assert_eq!(replay_json["stream"].as_array().unwrap().len(), 6);
    assert_eq!(replay_json["stream"][0]["packet"], "data");
    assert_eq!(replay_json["stream"][0]["kind"], "openai-gateway-event");
    assert_eq!(
        replay_json["stream"][0]["payload"]["event-kind"],
        "request-start"
    );
}

#[test]
fn response_sim_requires_admin_or_extension_capability() {
    let routes = routes_with_sim_keys();
    let response_id = stored_response_id(&routes, "inspect me");
    let path = format!("{RESPONSES_PATH}/{response_id}/sim");

    let denied = routes.handle(&GatewayRequest::get(path.clone()));
    let denied_json = response_json(&denied);
    assert_eq!(denied.status(), 403);
    assert_eq!(denied_json["error"]["code"], "capability_denied");

    let header_only = routes.handle(&header_sim_get(&path));
    assert_eq!(header_only.status(), 403);

    let allowed = routes.handle(&admin_get(&path));
    let allowed_json = response_json(&allowed);
    assert_eq!(allowed.status(), 200);
    assert_eq!(allowed_json["object"], "sim.gateway.response");
    assert_eq!(allowed_json["response_id"], response_id);
    assert_eq!(allowed_json["events"].as_array().unwrap().len(), 6);
    assert!(
        allowed_json["request"]["id"]
            .as_str()
            .unwrap()
            .starts_with("gwreq_")
    );
}

#[test]
fn replay_routes_enforce_owner_scope_and_redact_without_inspection() {
    let routes = routes_with_sim_keys();
    let response_id =
        stored_response_id_with_secret(&routes, EXTENSION_SECRET, "sensitive replay input");
    let events_path = format!("{RESPONSES_PATH}/{response_id}/events");

    let unauthenticated = routes.handle(&GatewayRequest::get(events_path.clone()));
    assert_eq!(unauthenticated.status(), 403);

    let other_owner = routes.handle(&authorized_get(OTHER_EXTENSION_SECRET, &events_path));
    assert_eq!(other_owner.status(), 403);

    let owner_events = routes.handle(&authorized_get(EXTENSION_SECRET, &events_path));
    let owner_events_json = response_json(&owner_events);
    assert_eq!(owner_events.status(), 200);
    assert_eq!(owner_events_json["data"][0]["payload"], "[redacted]");
    assert!(
        !owner_events_json
            .to_string()
            .contains("sensitive replay input")
    );

    let owner_replay = routes.handle(&authorized_json_request(
        EXTENSION_SECRET,
        "POST",
        SIM_REPLAY_PATH,
        &json!({ "response_id": response_id }).to_string(),
    ));
    let owner_replay_json = response_json(&owner_replay);
    assert_eq!(owner_replay.status(), 200);
    assert_eq!(owner_replay_json["stream"].as_array().unwrap().len(), 0);
    assert!(
        !owner_replay_json
            .to_string()
            .contains("sensitive replay input")
    );

    let inspected = routes.handle(&authorized_get(INSPECT_SECRET, &events_path));
    let inspected_json = response_json(&inspected);
    assert_eq!(inspected.status(), 200);
    assert_ne!(inspected_json["data"][0]["payload"], "[redacted]");
}

#[test]
fn fork_requires_authorized_response_owner_or_inspection() {
    let routes = routes_with_sim_keys();
    let parent_id = stored_response_id_with_secret(&routes, EXTENSION_SECRET, "fork owner input");

    let other_owner = routes.handle(&authorized_json_request(
        OTHER_EXTENSION_SECRET,
        "POST",
        SIM_FORK_PATH,
        &json!({
            "response_id": parent_id.clone(),
            "patch": { "model": "fixture/slow-echo" },
        })
        .to_string(),
    ));
    assert_eq!(other_owner.status(), 403);

    let owner = routes.handle(&authorized_json_request(
        EXTENSION_SECRET,
        "POST",
        SIM_FORK_PATH,
        &json!({
            "response_id": parent_id,
            "patch": { "model": "fixture/slow-echo" },
        })
        .to_string(),
    ));
    assert_eq!(owner.status(), 200);
}

#[test]
fn sim_fork_replays_original_input_with_changed_model_and_parent_link() {
    let routes = routes_with_sim_keys();
    let parent_id = stored_response_id(&routes, "fork input");
    let parent_sim =
        response_json(&routes.handle(&admin_get(&format!("{RESPONSES_PATH}/{parent_id}/sim"))));

    let fork = routes.handle(&authorized_json_request(
        ADMIN_SECRET,
        "POST",
        SIM_FORK_PATH,
        &json!({
            "response_id": parent_id.clone(),
            "patch": { "model": "fixture/slow-echo" },
        })
        .to_string(),
    ));
    let fork_json = response_json(&fork);

    assert_eq!(fork.status(), 200);
    assert_eq!(fork_json["object"], "sim.fork");
    assert_eq!(fork_json["parent_response_id"], parent_id);
    assert_eq!(fork_json["response"]["model"], "fixture/slow-echo");
    assert_eq!(fork_json["response"]["output_text"], "fork input");
    assert_ne!(fork_json["request_id"], parent_sim["request"]["id"]);
    assert_ne!(
        fork_json["request_content_id"],
        parent_sim["request_content_id"]
    );

    let fork_sim = response_json(&routes.handle(&admin_get(&format!(
        "{RESPONSES_PATH}/{}/sim",
        fork_json["response_id"].as_str().unwrap()
    ))));
    assert_eq!(fork_sim["parent_response_id"], parent_id);
}

fn stored_response_id(routes: &crate::GatewayRoutes, input: &str) -> String {
    stored_response_id_with_secret(routes, ADMIN_SECRET, input)
}

fn stored_response_id_with_secret(routes: &GatewayRoutes, secret: &str, input: &str) -> String {
    let response = routes.handle(&responses_request(
        &json!({
            "model": "fixture/echo",
            "input": input,
            "store": true,
        }),
        secret,
    ));
    assert_eq!(response.status(), 200);
    response_json(&response)["id"].as_str().unwrap().to_owned()
}

fn responses_request(body: &Value, secret: &str) -> GatewayRequest {
    authorized_json_request(secret, "POST", RESPONSES_PATH, &body.to_string())
}

fn json_request(method: &str, path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        method,
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn admin_get(path: &str) -> GatewayRequest {
    authorized_get(ADMIN_SECRET, path)
}

fn authorized_get(secret: &str, path: &str) -> GatewayRequest {
    GatewayRequest::new(
        "GET",
        path,
        vec![("Authorization".to_owned(), format!("Bearer {secret}"))],
        Vec::new(),
    )
}

fn authorized_json_request(secret: &str, method: &str, path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        method,
        path,
        vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), format!("Bearer {secret}")),
        ],
        body.as_bytes().to_vec(),
    )
}

fn header_sim_get(path: &str) -> GatewayRequest {
    GatewayRequest::new(
        "GET",
        path,
        vec![(
            "X-SIM-Capability".to_owned(),
            SIM_EXTENSION_CAPABILITY.to_owned(),
        )],
        Vec::new(),
    )
}

fn routes_with_sim_keys() -> GatewayRoutes {
    let key_table = OpenAiKeyTable::new().unwrap();
    key_table
        .add_secret(
            ADMIN_SECRET,
            CapabilitySet::new().grant(openai_gateway_admin_capability()),
        )
        .unwrap();
    for secret in [EXTENSION_SECRET, OTHER_EXTENSION_SECRET] {
        key_table
            .add_secret(
                secret,
                CapabilitySet::new()
                    .grant(sim_kernel::CapabilityName::new(SIM_EXTENSION_CAPABILITY)),
            )
            .unwrap();
    }
    key_table
        .add_secret(
            INSPECT_SECRET,
            CapabilitySet::new().grant(sim_kernel::CapabilityName::new(SIM_INSPECTION_CAPABILITY)),
        )
        .unwrap();
    configure_routes_with_state(GatewayRouteState::memory().with_keys(key_table))
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn event_ids(response: &Value) -> Vec<String> {
    response["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["id"].as_str().unwrap().to_owned())
        .collect()
}
