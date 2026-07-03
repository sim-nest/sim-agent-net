use serde_json::Value;

use crate::{
    FILES_PATH, GatewayRequest, GatewayStateStore, THREAD_MESSAGES_ROUTE, THREADS_PATH,
    configure_routes,
};

#[test]
fn files_route_stores_json_upload_and_fetches_metadata() {
    let routes = configure_routes();
    let uploaded = routes.handle(&json_request(
        "POST",
        FILES_PATH,
        r#"{"filename":"notes.txt","purpose":"assistants","content":"hello file"}"#,
    ));
    let uploaded_json = response_json(&uploaded);
    let file_id = uploaded_json["id"].as_str().unwrap();

    assert_eq!(uploaded.status(), 200);
    assert!(file_id.starts_with("file_"));
    assert_eq!(uploaded_json["object"], "file");
    assert_eq!(uploaded_json["filename"], "notes.txt");
    assert_eq!(uploaded_json["bytes"], 10);
    assert_eq!(uploaded_json["storage_ref"]["kind"], "memory");

    let fetched = routes.handle(&GatewayRequest::get(format!("{FILES_PATH}/{file_id}")));
    assert_eq!(fetched.status(), 200);
    assert_eq!(response_json(&fetched), uploaded_json);
    let store = routes.state().store().lock().unwrap();
    assert_eq!(store.file_bytes(file_id).unwrap(), b"hello file");
}

#[test]
fn files_route_records_table_fs_storage_ref_metadata() {
    let uploaded = configure_routes().handle(&json_request(
        "POST",
        FILES_PATH,
        r#"{"filename":"fixture.txt","content":"table ref","storage_ref":{"kind":"table-fs","path":"fixtures/upload.txt"}}"#,
    ));
    let json = response_json(&uploaded);

    assert_eq!(uploaded.status(), 200);
    assert_eq!(json["storage_ref"]["kind"], "table-fs");
    assert_eq!(json["storage_ref"]["path"], "fixtures/upload.txt");
}

#[test]
fn threads_route_creates_reads_appends_and_lists_messages() {
    let routes = configure_routes();
    let created = routes.handle(&json_request(
        "POST",
        THREADS_PATH,
        r#"{"metadata":{"case":"stateful"}}"#,
    ));
    let created_json = response_json(&created);
    let thread_id = created_json["id"].as_str().unwrap();

    assert_eq!(created.status(), 200);
    assert!(thread_id.starts_with("thread_"));
    assert_eq!(created_json["object"], "thread");
    assert_eq!(created_json["metadata"]["case"], "stateful");

    let fetched = routes.handle(&GatewayRequest::get(format!("{THREADS_PATH}/{thread_id}")));
    assert_eq!(response_json(&fetched), created_json);

    let message_path = format!("{THREADS_PATH}/{thread_id}/messages");
    let appended = routes.handle(&json_request(
        "POST",
        &message_path,
        r#"{"role":"user","content":"stored context"}"#,
    ));
    let appended_json = response_json(&appended);
    assert_eq!(appended.status(), 200);
    assert_eq!(appended_json["object"], "thread.message");
    assert_eq!(appended_json["thread_id"], thread_id);
    assert_eq!(appended_json["content"], "stored context");

    let listed = routes.handle(&GatewayRequest::get(message_path));
    let listed_json = response_json(&listed);
    assert_eq!(listed.status(), 200);
    assert_eq!(listed_json["object"], "list");
    assert_eq!(listed_json["data"][0], appended_json);
}

#[test]
fn responses_route_can_include_prior_thread_messages() {
    let routes = configure_routes();
    let thread = routes.handle(&json_request("POST", THREADS_PATH, "{}"));
    let thread_id = response_json(&thread)["id"].as_str().unwrap().to_owned();
    let message_path = format!("{THREADS_PATH}/{thread_id}/messages");
    routes.handle(&json_request(
        "POST",
        &message_path,
        r#"{"role":"user","content":"stored context"}"#,
    ));

    let response = routes.handle(&json_request(
        "POST",
        crate::RESPONSES_PATH,
        &format!(r#"{{"model":"fixture/echo","input":"new question","thread_id":"{thread_id}"}}"#),
    ));
    let response_json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(response_json["output_text"], "new question stored context");
}

fn json_request(method: &str, path: &str, body: &str) -> GatewayRequest {
    GatewayRequest::new(
        method,
        path,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn response_json(response: &crate::GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

#[test]
fn thread_messages_route_constant_is_stable() {
    assert_eq!(THREAD_MESSAGES_ROUTE, "/v1/threads/{id}/messages");
}
