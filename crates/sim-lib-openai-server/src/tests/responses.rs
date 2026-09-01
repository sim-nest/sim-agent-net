use serde_json::Value;
#[cfg(feature = "http")]
use sim_codec_binary::BinaryCodecLib;
use sim_kernel::Expr;
#[cfg(feature = "http")]
use sim_kernel::{Symbol, eval_fabric_capability};

use crate::{
    DeterministicWallClock, GatewayEvent, GatewayRequest, GatewayResponseObjectStore,
    GatewayResponseValue, GatewayStore, MemoryGatewayStore, OpenAiGatewayFabric, RESPONSES_PATH,
    ResponseIdGenerators, configure_routes, execute_response_request,
    gateway_event_data_from_packet, gateway_event_data_packets, openai_gateway_plan_capability,
};
#[cfg(feature = "http")]
use crate::{fabric_symbol, install_openai_gateway_lib};

#[test]
fn responses_missing_model_uses_shared_missing_required_error() {
    let response = configure_routes().handle(&responses_request("{}"));
    assert_eq!(response.status(), 400);
    let error = response_json(&response);
    assert_eq!(error["error"]["code"], "missing_required_parameter");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("model")
    );
}

#[test]
fn responses_route_fixture_echo_returns_openai_response_object() {
    let response = configure_routes().handle(&responses_request(
        r#"{"model":"fixture/echo","input":"hello","store":false}"#,
    ));
    let json = response_json(&response);

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(json["object"], "response");
    assert!(json["id"].as_str().unwrap().starts_with("resp_"));
    assert_eq!(json["status"], "completed");
    assert_eq!(json["model"], "fixture/echo");
    assert_eq!(json["output_text"], "hello");
    assert_eq!(json["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(json["usage"]["total_tokens"], 2);
}

#[test]
fn responses_route_streams_sse_and_stores_final_response() {
    let routes = configure_routes();
    let response = routes.handle(&responses_request(
        r#"{"model":"fixture/echo","input":"stream me","store":true,"stream":true}"#,
    ));

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("text/event-stream"));
    assert!(super::sse_ends_with_done(&response));
    let chunks = super::sse_json_chunks(&response);
    let text = response_stream_text(&chunks);
    assert_eq!(text, "stream me");

    let final_chunk = chunks
        .iter()
        .find(|chunk| chunk["type"] == "response.completed")
        .unwrap();
    let response_id = final_chunk["response"]["id"].as_str().unwrap();
    let retrieved = routes.handle(&retrieval_request(response_id));
    let stored = response_json(&retrieved);
    assert_eq!(stored["output_text"], text);
}

#[test]
fn gateway_fabric_realize_returns_response_without_http() {
    let mut cx = super::cx();
    let fabric = OpenAiGatewayFabric::deterministic(1, 1_000, 10);
    let gateway_request =
        responses_request(r#"{"model":"fixture/echo","input":"fabric response","store":false}"#);

    let reply = sim_kernel::realize_final(
        &mut cx,
        &fabric,
        OpenAiGatewayFabric::eval_request_for_gateway_request(&gateway_request),
    )
    .unwrap();
    let response = reply
        .value
        .object()
        .downcast_ref::<GatewayResponseValue>()
        .unwrap()
        .response();
    let json = response_json(response);

    assert_eq!(response.status(), 200);
    assert_eq!(json["output_text"], "fabric response");
}

#[test]
fn gateway_fabric_and_http_execution_share_event_log() {
    let request = responses_request(
        r#"{"model":"fixture/echo","input":"shared event log","store":true,"stream":true}"#,
    );
    let mut http_cx = super::cx();
    let mut http_store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(7);
    let mut clock = DeterministicWallClock::new(2_000, 20);
    let http_execution = execute_response_request(
        &mut http_cx,
        &mut http_store,
        &mut ids,
        &mut clock,
        &request,
    );

    let mut fabric_cx = super::cx();
    let fabric = OpenAiGatewayFabric::deterministic(7, 2_000, 20);
    let reply = sim_kernel::realize_final(
        &mut fabric_cx,
        &fabric,
        OpenAiGatewayFabric::eval_request_for_gateway_request(&request),
    )
    .unwrap();
    let fabric_response = reply
        .value
        .object()
        .downcast_ref::<GatewayResponseValue>()
        .unwrap()
        .response()
        .clone();
    let fabric_execution = fabric.last_execution().unwrap().unwrap();

    assert_eq!(&fabric_response, http_execution.response());
    assert_eq!(fabric_execution.events(), http_execution.events());
    assert_eq!(
        fabric_execution.event_content_ids(),
        http_execution.event_content_ids()
    );
    assert_eq!(
        gateway_event_data_packets(fabric_execution.events()),
        gateway_event_data_packets(http_execution.events())
    );
}

#[test]
fn streaming_response_data_packets_reconstruct_event_log() {
    let request = responses_request(
        r#"{"model":"fixture/echo","input":"packet stream","store":true,"stream":true}"#,
    );
    let mut cx = super::cx();
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(17);
    let mut clock = DeterministicWallClock::new(4_000, 40);

    let execution = execute_response_request(&mut cx, &mut store, &mut ids, &mut clock, &request);
    let packets = gateway_event_data_packets(execution.events());
    let data = packets
        .iter()
        .map(gateway_event_data_from_packet)
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();

    assert_eq!(
        execution.response().header("Content-Type"),
        Some("text/event-stream")
    );
    assert_eq!(
        response_stream_text(&super::sse_json_chunks(execution.response())),
        "packet stream"
    );
    assert_eq!(data.len(), execution.events().len());
    for (data, event) in data.iter().zip(execution.events()) {
        assert_eq!(data.sequence(), event.sequence());
        assert_eq!(data.kind(), event.kind());
        assert_eq!(data.payload(), event.payload());
    }
}

#[cfg(feature = "http")]
#[test]
fn server_realize_drives_gateway_fabric_value() {
    let mut cx = sim_kernel::Cx::new(
        std::sync::Arc::new(sim_kernel::EagerPolicy),
        std::sync::Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(0x0A11_CE10),
    );
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    sim_lib_server::install_server_lib(&mut cx).unwrap();
    install_openai_gateway_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let response = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                Expr::Bytes(
                    br#"{"model":"fixture/echo","input":"server realize","store":false}"#.to_vec(),
                ),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(fabric_symbol())),
                    args: Vec::new(),
                },
            ],
        })
        .unwrap();
    let response_expr = response.object().as_expr(&mut cx).unwrap();
    let body = match required_expr_field(&response_expr, "body") {
        Expr::Bytes(body) => body,
        other => panic!("gateway response body should be bytes, found {other:?}"),
    };
    let json: Value = serde_json::from_slice(body).unwrap();

    assert_eq!(
        required_expr_field(&response_expr, "status"),
        &Expr::String("200".to_owned())
    );
    assert_eq!(json["output_text"], "server realize");
}

#[cfg(feature = "http")]
#[test]
fn gateway_eval_site_answer_delegates_to_fabric_realize() {
    use sim_lib_server::EvalSite;

    let mut cx = sim_kernel::Cx::new(
        std::sync::Arc::new(sim_kernel::NoopEvalPolicy),
        std::sync::Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(0x0A11_CE11),
    );
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    let fabric = OpenAiGatewayFabric::deterministic(11, 3_000, 30);
    let request =
        responses_request(r#"{"model":"fixture/echo","input":"eval site answer","store":true}"#);

    assert_eq!(fabric.site_kind(), "openai-gateway");
    assert_eq!(fabric.address(), &sim_lib_server::ServerAddress::Local);
    assert!(fabric.as_eval_fabric().is_some());

    let frame = sim_lib_server::server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        OpenAiGatewayFabric::eval_request_for_gateway_request(&request),
    )
    .unwrap();
    let reply_frame = fabric.answer(&mut cx, frame).unwrap();
    let reply = sim_lib_server::eval_reply_from_frame(&mut cx, &reply_frame).unwrap();
    let response_expr = reply.value.object().as_expr(&mut cx).unwrap();
    let body = match required_expr_field(&response_expr, "body") {
        Expr::Bytes(body) => body,
        other => panic!("gateway response body should be bytes, found {other:?}"),
    };
    let json: Value = serde_json::from_slice(body).unwrap();

    assert_eq!(json["output_text"], "eval site answer");
    assert_eq!(fabric.last_execution().unwrap().unwrap().events().len(), 6);
}

#[test]
fn response_execution_stores_response_when_requested() {
    let mut cx = super::cx();
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicWallClock::new(1_000, 10);
    let request = responses_request(r#"{"model":"fixture/echo","input":"store me","store":true}"#);

    let execution = execute_response_request(&mut cx, &mut store, &mut ids, &mut clock, &request);

    assert_eq!(execution.response().status(), 200);
    assert!(execution.request_content_id().is_some());
    assert!(execution.run_content_id().is_some());
    assert_eq!(execution.event_content_ids().len(), 6);
    let response_content_id = execution.response_content_id().unwrap();
    assert_eq!(
        store.response(response_content_id),
        Some(execution.response().clone())
    );
    let response_id = execution.response_id().unwrap();
    let record = store.response_object(response_id).unwrap();
    assert_eq!(record.content_id(), response_content_id);
    assert_eq!(record.response(), execution.response());
}

#[test]
fn response_stream_execution_stores_split_event_log() {
    let mut cx = super::cx();
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicWallClock::new(1_000, 10);
    let request = responses_request(
        r#"{"model":"fixture/echo","input":"split me now","store":true,"stream":true}"#,
    );

    let execution = execute_response_request(&mut cx, &mut store, &mut ids, &mut clock, &request);

    assert_eq!(execution.response().status(), 200);
    assert_eq!(
        execution.response().header("Content-Type"),
        Some("text/event-stream")
    );
    assert_eq!(
        stored_events(&store, execution.event_content_ids()),
        execution.events()
    );
    let deltas = execution
        .events()
        .iter()
        .filter(|event| event.kind().name.as_ref() == "delta")
        .map(|event| match event.payload() {
            Expr::String(text) => text.as_str(),
            other => panic!("delta payload should be string, found {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(deltas, vec!["split", " me", " now"]);

    let record = store
        .response_object(execution.response_id().unwrap())
        .unwrap();
    let stored = response_json(record.response());
    assert_eq!(stored["output_text"], "split me now");
}

#[test]
fn responses_execution_records_plan_branch_events() {
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    let mut store = MemoryGatewayStore::new();
    let mut ids = ResponseIdGenerators::deterministic(1);
    let mut clock = DeterministicWallClock::new(1_000, 10);
    let request = responses_request(
        r#"{"model":"race(fixture/echo, fixture/slow-echo)","input":"branch me","store":true}"#,
    );

    let execution = execute_response_request(&mut cx, &mut store, &mut ids, &mut clock, &request);

    assert_eq!(execution.response().status(), 200);
    assert!(
        execution
            .events()
            .iter()
            .any(|event| event.kind().name.as_ref() == "branch-start")
    );
    assert!(
        execution
            .events()
            .iter()
            .any(|event| event.kind().name.as_ref() == "branch-end"
                && format!("{:?}", event.payload()).contains("cancelled"))
    );
}

#[test]
fn responses_route_retrieves_stored_response_by_id() {
    let routes = configure_routes();
    let stored = routes.handle(&responses_request(
        r#"{"model":"fixture/echo","input":"stored route","store":true}"#,
    ));
    let stored_json = response_json(&stored);
    let response_id = stored_json["id"].as_str().unwrap();

    let retrieved = routes.handle(&retrieval_request(response_id));

    assert_eq!(retrieved.status(), 200);
    assert_eq!(retrieved.body(), stored.body());
}

#[test]
fn responses_route_retrieval_reports_not_found() {
    let routes = configure_routes();

    let unknown = routes.handle(&retrieval_request("resp_missing"));
    let unknown_json = response_json(&unknown);
    assert_eq!(unknown.status(), 404);
    assert_eq!(unknown_json["error"]["param"], "id");
    assert_eq!(unknown_json["error"]["code"], "not_found");

    let unstored = routes.handle(&responses_request(
        r#"{"model":"fixture/echo","input":"not stored","store":false}"#,
    ));
    let unstored_json = response_json(&unstored);
    let unstored_id = unstored_json["id"].as_str().unwrap();
    let missing = routes.handle(&retrieval_request(unstored_id));
    let missing_json = response_json(&missing);
    assert_eq!(missing.status(), 404);
    assert_eq!(missing_json["error"]["code"], "not_found");
}

#[test]
fn responses_route_returns_structured_errors() {
    let missing_model = configure_routes().handle(&responses_request(r#"{"input":"hello"}"#));
    let missing_model_json = response_json(&missing_model);
    assert_eq!(missing_model.status(), 400);
    assert_eq!(missing_model_json["error"]["param"], "model");
    assert_eq!(
        missing_model_json["error"]["code"],
        "missing_required_parameter"
    );

    let missing_input = configure_routes().handle(&responses_request(
        r#"{"model":"fixture/echo","store":true}"#,
    ));
    let missing_input_json = response_json(&missing_input);
    assert_eq!(missing_input.status(), 400);
    assert_eq!(missing_input_json["error"]["param"], "input");
    assert_eq!(
        missing_input_json["error"]["code"],
        "missing_required_parameter"
    );

    let unknown_model = configure_routes().handle(&responses_request(
        r#"{"model":"missing/model","input":"hello","store":true}"#,
    ));
    let unknown_model_json = response_json(&unknown_model);
    assert_eq!(unknown_model.status(), 404);
    assert_eq!(unknown_model_json["error"]["param"], "model");
    assert_eq!(unknown_model_json["error"]["code"], "model_not_found");
}

fn responses_request(body: &str) -> GatewayRequest {
    GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body.as_bytes().to_vec(),
    )
}

fn retrieval_request(response_id: &str) -> GatewayRequest {
    GatewayRequest::get(format!("{RESPONSES_PATH}/{response_id}"))
}

fn response_json(response: &crate::GatewayResponse) -> Value {
    serde_json::from_slice(response.body()).unwrap()
}

fn response_stream_text(chunks: &[Value]) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk["type"] == "response.output_text.delta")
        .map(|chunk| chunk["delta"].as_str().unwrap())
        .collect()
}

fn stored_events(store: &MemoryGatewayStore, ids: &[sim_kernel::ContentId]) -> Vec<GatewayEvent> {
    ids.iter().map(|id| store.event(id).unwrap()).collect()
}

#[cfg(feature = "http")]
fn required_expr_field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map expression, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
                Some(value)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected field {name} in {expr:?}"))
}
