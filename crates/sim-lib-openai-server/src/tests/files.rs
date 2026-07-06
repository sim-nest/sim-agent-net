use serde_json::Value;

use crate::{FILES_PATH, GatewayRequest, GatewayResponse, configure_routes};

#[test]
fn files_missing_filename_uses_shared_missing_required_error() {
    let response = configure_routes().handle(&json_request(FILES_PATH, "{}"));
    assert_eq!(response.status(), 400);
    let error = response_json(&response);
    assert_eq!(error["error"]["code"], "missing_required_parameter");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("filename")
    );
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
