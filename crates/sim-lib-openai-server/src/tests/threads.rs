use serde_json::Value;

use crate::{GatewayRequest, GatewayResponse, THREADS_PATH, configure_routes};

#[test]
fn thread_creation_accepts_empty_body_as_empty_object() {
    let routes = configure_routes();
    let response = routes.handle(&GatewayRequest::new("POST", THREADS_PATH, vec![], vec![]));

    assert_eq!(response.status(), 200);
    let json = response_json(&response);
    assert_eq!(json["object"], "thread");
    assert_eq!(json["metadata"], serde_json::json!({}));
}

#[test]
fn thread_creation_accepts_json_null_as_empty_object() {
    let routes = configure_routes();
    let response = routes.handle(&json_request(THREADS_PATH, "null"));

    assert_eq!(response.status(), 200);
    let json = response_json(&response);
    assert_eq!(json["object"], "thread");
    assert_eq!(json["metadata"], serde_json::json!({}));
}

#[test]
fn thread_message_missing_role_uses_shared_missing_required_error() {
    let routes = configure_routes();
    let thread = response_json(&routes.handle(&json_request(THREADS_PATH, "{}")));
    let thread_id = thread["id"].as_str().unwrap();

    let response = routes.handle(&json_request(
        &format!("{THREADS_PATH}/{thread_id}/messages"),
        "{}",
    ));
    assert_eq!(response.status(), 400);
    let error = response_json(&response);
    assert_eq!(error["error"]["code"], "missing_required_parameter");
    assert!(error["error"]["message"].as_str().unwrap().contains("role"));
}

fn json_request(path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}
