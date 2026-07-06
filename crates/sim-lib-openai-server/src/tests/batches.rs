use serde_json::{Value, json};

use crate::{
    BATCH_CANCEL_ROUTE, BATCHES_PATH, FILES_PATH, GatewayRequest, GatewayResponse,
    GatewayResponseObjectStore, GatewayStateStore, RESPONSES_PATH, configure_routes,
};

#[test]
fn batches_missing_input_file_id_uses_shared_missing_required_error() {
    let created = configure_routes().handle(&json_request("POST", BATCHES_PATH, "{}"));
    assert_eq!(created.status(), 400);
    let error = response_json(&created);
    assert_eq!(error["error"]["code"], "missing_required_parameter");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("input_file_id")
    );
}

#[test]
fn batches_route_completes_fixture_responses_and_writes_output_file() {
    let routes = configure_routes();
    let input_file_id = upload_batch_file(
        &routes,
        &batch_jsonl(&[("req-1", "first item"), ("req-2", "second item")]),
    );

    let created = routes.handle(&json_request(
        "POST",
        BATCHES_PATH,
        &json!({
            "input_file_id": input_file_id,
            "endpoint": RESPONSES_PATH,
        })
        .to_string(),
    ));
    let created_json = response_json(&created);

    assert_eq!(created.status(), 200);
    assert_eq!(created_json["object"], "batch");
    assert_eq!(created_json["status"], "completed");
    assert_eq!(created_json["request_counts"]["total"], 2);
    assert_eq!(created_json["request_counts"]["completed"], 2);
    assert_eq!(created_json["request_counts"]["failed"], 0);

    let output_file_id = created_json["output_file_id"].as_str().unwrap();
    let output = stored_jsonl(&routes, output_file_id);
    assert_eq!(output.len(), 2);
    assert_eq!(output[0]["custom_id"], "req-1");
    assert_eq!(output[0]["response"]["body"]["output_text"], "first item");
    assert_eq!(output[1]["response"]["body"]["output_text"], "second item");

    let response_id = output[0]["response"]["body"]["id"].as_str().unwrap();
    let store = routes.state().store().lock().unwrap();
    assert!(store.response_object(response_id).is_some());
}

#[test]
fn batches_route_writes_error_file_for_invalid_jsonl_line() {
    let routes = configure_routes();
    let input_file_id = upload_batch_file(
        &routes,
        &format!("{}\nnot json\n", batch_line("ok", "valid item")),
    );

    let created = routes.handle(&json_request(
        "POST",
        BATCHES_PATH,
        &json!({
            "input_file_id": input_file_id,
            "endpoint": RESPONSES_PATH,
        })
        .to_string(),
    ));
    let created_json = response_json(&created);

    assert_eq!(created.status(), 200);
    assert_eq!(created_json["status"], "completed");
    assert_eq!(created_json["request_counts"]["completed"], 1);
    assert_eq!(created_json["request_counts"]["failed"], 1);

    let error_file_id = created_json["error_file_id"].as_str().unwrap();
    let errors = stored_jsonl(&routes, error_file_id);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["line"], 2);
    assert_eq!(errors[0]["error"]["code"], "invalid_json");
}

#[test]
fn batch_cancel_moves_deferred_queued_work_to_cancelled() {
    let routes = configure_routes();
    let input_file_id = upload_batch_file(&routes, &batch_jsonl(&[("later-1", "later")]));
    let queued = routes.handle(&json_request(
        "POST",
        BATCHES_PATH,
        &json!({
            "input_file_id": input_file_id,
            "endpoint": RESPONSES_PATH,
            "defer": true,
        })
        .to_string(),
    ));
    let queued_json = response_json(&queued);
    let batch_id = queued_json["id"].as_str().unwrap();

    assert_eq!(queued.status(), 200);
    assert_eq!(queued_json["status"], "queued");

    let cancel_path = BATCH_CANCEL_ROUTE.replace("{id}", batch_id);
    let cancelled = routes.handle(&json_request("POST", &cancel_path, "{}"));
    let cancelled_json = response_json(&cancelled);

    assert_eq!(cancelled.status(), 200);
    assert_eq!(cancelled_json["status"], "cancelled");
    assert_eq!(cancelled_json["request_counts"]["cancelled"], 1);

    let fetched = routes.handle(&GatewayRequest::get(format!("{BATCHES_PATH}/{batch_id}")));
    assert_eq!(response_json(&fetched), cancelled_json);
}

fn batch_jsonl(items: &[(&str, &str)]) -> String {
    let mut text = items
        .iter()
        .map(|(custom_id, input)| batch_line(custom_id, input))
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

fn batch_line(custom_id: &str, input: &str) -> String {
    json!({
        "custom_id": custom_id,
        "method": "POST",
        "url": RESPONSES_PATH,
        "body": {
            "model": "fixture/echo",
            "input": input,
            "store": false,
        },
    })
    .to_string()
}

fn upload_batch_file(routes: &crate::GatewayRoutes, content: &str) -> String {
    let response = routes.handle(&json_request(
        "POST",
        FILES_PATH,
        &json!({
            "filename": "batch.jsonl",
            "purpose": "batch",
            "content": content,
        })
        .to_string(),
    ));
    assert_eq!(response.status(), 200);
    response_json(&response)["id"].as_str().unwrap().to_owned()
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

fn stored_jsonl(routes: &crate::GatewayRoutes, file_id: &str) -> Vec<Value> {
    let store = routes.state().store().lock().unwrap();
    let bytes = store.file_bytes(file_id).unwrap();
    std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
