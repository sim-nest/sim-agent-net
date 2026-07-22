use sim_kernel::{Expr, Symbol};

use crate::{
    DeterministicGatewayClock, GATEWAY_EVENT_OBJECT, GATEWAY_REQUEST_OBJECT,
    GATEWAY_RESPONSE_OBJECT, GATEWAY_RUN_OBJECT, GatewayBatch, GatewayBatchCounts, GatewayClock,
    GatewayEvent, GatewayFile, GatewayFileStorageRef, GatewayIdGenerator, GatewayRequest,
    GatewayResponse, GatewayResponseObjectStore, GatewayRun, GatewayStateStore, GatewayStore,
    GatewayThread, GatewayThreadMessage, MemoryGatewayStore, StoredGatewayResponse,
    TableGatewayStore, content_id_for_expr, request_content_id,
};

#[test]
fn request_content_id_ignores_volatile_id_and_timestamp() {
    let left = GatewayRequest::new(
        "POST",
        "/v1/responses",
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"fixture/echo","input":"same"}"#.to_vec(),
    )
    .with_metadata("req_000001", 1_000);
    let right = GatewayRequest::new(
        "POST",
        "/v1/responses",
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"model":"fixture/echo","input":"same"}"#.to_vec(),
    )
    .with_metadata("req_000999", 9_999);

    assert_eq!(
        request_content_id(&left).unwrap(),
        request_content_id(&right).unwrap()
    );
}

#[test]
fn request_content_id_changes_when_prompt_changes() {
    let left = GatewayRequest::new(
        "POST",
        "/v1/responses",
        Vec::new(),
        br#"{"model":"fixture/echo","input":"alpha"}"#.to_vec(),
    );
    let right = GatewayRequest::new(
        "POST",
        "/v1/responses",
        Vec::new(),
        br#"{"model":"fixture/echo","input":"beta"}"#.to_vec(),
    );

    assert_ne!(
        request_content_id(&left).unwrap(),
        request_content_id(&right).unwrap()
    );
}

#[test]
fn gateway_objects_project_to_stable_maps() {
    let request = GatewayRequest::new(
        "POST",
        "/v1/responses",
        vec![
            ("X-Trace".to_owned(), "b".to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ],
        b"{}".to_vec(),
    )
    .with_metadata("req_000001", 10);
    let request_expr = request.to_expr();

    assert_eq!(
        map_field(&request_expr, "object"),
        Some(&Expr::String(GATEWAY_REQUEST_OBJECT.to_owned()))
    );
    let Expr::List(headers) = map_field(&request_expr, "headers").unwrap() else {
        panic!("headers must project as a list");
    };
    assert_eq!(
        map_field(&headers[0], "name"),
        Some(&Expr::String("Accept".to_owned()))
    );

    let response = GatewayResponse::text(404, "missing");
    assert_eq!(
        map_field(&response.to_expr(), "object"),
        Some(&Expr::String(GATEWAY_RESPONSE_OBJECT.to_owned()))
    );

    let run = GatewayRun::new("run_000001", request_content_id(&request).unwrap(), 20);
    assert_eq!(
        map_field(&run.to_expr(), "object"),
        Some(&Expr::String(GATEWAY_RUN_OBJECT.to_owned()))
    );

    let event = GatewayEvent::new(
        "evt_000001",
        run.id(),
        0,
        Symbol::new("response.created"),
        Expr::Nil,
        30,
    );
    assert_eq!(
        map_field(&event.to_expr(), "object"),
        Some(&Expr::String(GATEWAY_EVENT_OBJECT.to_owned()))
    );
}

#[test]
fn deterministic_gateway_ids_and_clock_are_reproducible() {
    let mut ids = GatewayIdGenerator::deterministic("run", 7);
    let mut clock = DeterministicGatewayClock::new(1_000, 25);

    assert_eq!(ids.next_id().unwrap(), "run_000007");
    assert_eq!(ids.next_id().unwrap(), "run_000008");
    assert_eq!(clock.now_ms().unwrap(), 1_000);
    assert_eq!(clock.now_ms().unwrap(), 1_025);
}

#[test]
fn memory_gateway_store_round_trips_each_gateway_object() {
    let mut store = MemoryGatewayStore::new();
    assert_gateway_store_round_trip(&mut store);
}

#[test]
fn table_gateway_store_round_trips_each_gateway_object() {
    let mut store = TableGatewayStore::new().unwrap();
    assert_gateway_store_round_trip(&mut store);
}

#[test]
fn table_gateway_store_round_trips_response_and_state_records() {
    let mut store = TableGatewayStore::new().unwrap();
    assert_response_and_state_store_round_trip(&mut store);
}

fn assert_gateway_store_round_trip<S>(store: &mut S)
where
    S: GatewayStore,
{
    let mut ids = GatewayIdGenerator::deterministic("gw", 1);
    let mut clock = DeterministicGatewayClock::new(500, 10);
    let request = GatewayRequest::new(
        "POST",
        "/v1/responses",
        Vec::new(),
        br#"{"model":"fixture/echo","input":"store me"}"#.to_vec(),
    )
    .with_metadata(ids.next_id().unwrap(), clock.now_ms().unwrap());
    let request_id = request_content_id(&request).unwrap();
    store
        .put_request(request_id.clone(), request.clone())
        .unwrap();

    let run = GatewayRun::new(
        ids.next_id().unwrap(),
        request_id.clone(),
        clock.now_ms().unwrap(),
    );
    let run_id = content_id_for_expr(&run.to_expr()).unwrap();
    store.put_run(run_id.clone(), run.clone()).unwrap();
    let event = GatewayEvent::new(
        ids.next_id().unwrap(),
        run.id(),
        0,
        Symbol::new("response.created"),
        Expr::String("created".to_owned()),
        clock.now_ms().unwrap(),
    );
    let event_id = content_id_for_expr(&event.to_expr()).unwrap();
    store.put_event(event_id.clone(), event.clone()).unwrap();
    let response = GatewayResponse::json(200, br#"{"output_text":"store me"}"#.to_vec());
    let response_id = content_id_for_expr(&response.to_expr()).unwrap();
    store
        .put_response(response_id.clone(), response.clone())
        .unwrap();

    assert_eq!(store.request(&request_id), Some(request));
    assert_eq!(store.run(&run_id), Some(run));
    assert_eq!(store.event(&event_id), Some(event));
    assert_eq!(store.response(&response_id), Some(response));
}

fn assert_response_and_state_store_round_trip<S>(store: &mut S)
where
    S: GatewayStore + GatewayResponseObjectStore + GatewayStateStore,
{
    let mut ids = GatewayIdGenerator::deterministic("gw", 20);
    let mut clock = DeterministicGatewayClock::new(2_000, 10);
    let request = GatewayRequest::new(
        "POST",
        "/v1/responses",
        Vec::new(),
        br#"{"model":"fixture/echo","input":"state"}"#.to_vec(),
    )
    .with_metadata(ids.next_id().unwrap(), clock.now_ms().unwrap());
    let request_id = request_content_id(&request).unwrap();
    store.put_request(request_id.clone(), request).unwrap();
    let response = GatewayResponse::json(200, br#"{"output_text":"state"}"#.to_vec());
    let response_content_id = content_id_for_expr(&response.to_expr()).unwrap();
    let mut stored_response =
        StoredGatewayResponse::new("resp_state", response_content_id.clone(), response.clone());
    stored_response.request_content_id = Some(request_id.clone());
    stored_response.event_content_ids = vec![request_id.clone()];
    store.put_response_object(stored_response.clone()).unwrap();

    let file = GatewayFile::new(
        "file_state",
        "input.jsonl",
        "batch",
        5,
        clock.now_ms().unwrap(),
        GatewayFileStorageRef::memory(response_content_id.clone()),
    );
    store.put_file(file.clone(), b"hello".to_vec()).unwrap();
    let batch = GatewayBatch::new(
        "batch_state",
        file.id(),
        "/v1/responses",
        clock.now_ms().unwrap(),
        GatewayBatchCounts::new(1, 0, 0, 0),
    );
    store.put_batch(batch.clone()).unwrap();
    let thread = GatewayThread::new(
        "thread_state",
        clock.now_ms().unwrap(),
        vec![("topic".to_owned(), "storage".to_owned())],
    );
    store.put_thread(thread.clone()).unwrap();
    let first_message = GatewayThreadMessage::new(
        ids.next_id().unwrap(),
        thread.id(),
        "user",
        "hello",
        clock.now_ms().unwrap(),
    );
    let second_message = GatewayThreadMessage::new(
        ids.next_id().unwrap(),
        thread.id(),
        "assistant",
        "world",
        clock.now_ms().unwrap(),
    );
    store.put_thread_message(first_message.clone()).unwrap();
    store.put_thread_message(second_message.clone()).unwrap();

    assert_eq!(store.response(&response_content_id), Some(response));
    assert_eq!(store.response_object("resp_state"), Some(stored_response));
    assert_eq!(store.file("file_state"), Some(file));
    assert_eq!(store.file_bytes("file_state"), Some(b"hello".to_vec()));
    assert_eq!(store.batch("batch_state"), Some(batch));
    assert_eq!(store.thread("thread_state"), Some(thread));
    assert_eq!(
        store.thread_messages("thread_state"),
        vec![first_message, second_message]
    );
}

use sim_value::access::field as map_field;
