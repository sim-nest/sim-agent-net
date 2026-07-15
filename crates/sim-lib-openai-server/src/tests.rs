use std::sync::Arc;

use serde_json::Value;
use sim_codec::{Input, Output, decode_with_codec, encode_with_codec};
use sim_codec_chat::{
    ChatCodecLib, model_card_expr, model_response_expr, validate_chat_transcript,
};
use sim_kernel::{
    Args, Callable, CapabilityName, Cx, EncodeOptions, Error, Expr, Lib, Object, ObjectCompat,
    ReadPolicy, Result, Symbol,
};

use super::*;
mod admin;
mod batches;
mod cache;
mod chat_completions;
mod embeddings;
mod execution_record;
mod fabric;
mod federation;
mod files;
mod gateway_objects;
mod keys;
mod media_vector;
mod models;
mod plan;
mod replay;
mod responses;
mod stateful;
mod threads;
mod tools;
use sim_kernel::testing::bare_cx as cx;

fn sse_data_lines(response: &GatewayResponse) -> Vec<String> {
    std::str::from_utf8(response.body())
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: ").map(str::to_owned))
        .collect()
}

fn sse_json_chunks(response: &GatewayResponse) -> Vec<Value> {
    sse_data_lines(response)
        .into_iter()
        .filter(|line| line != "[DONE]")
        .map(|line| serde_json::from_str(&line).unwrap())
        .collect()
}

fn sse_ends_with_done(response: &GatewayResponse) -> bool {
    sse_data_lines(response)
        .last()
        .is_some_and(|line| line == "[DONE]")
}

#[test]
fn install_openai_gateway_lib_is_idempotent() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    install_openai_gateway_lib(&mut cx).unwrap();
    let manifest = &cx.registry().lib(&manifest_name()).unwrap().manifest;
    assert_eq!(manifest.id, manifest_name());
    assert_eq!(manifest.exports.len(), 19);
}

#[test]
fn manifest_exports_gateway_functions_and_openai_codec() {
    let manifest = OpenAiGatewayLib.manifest();
    assert_eq!(manifest.id, Symbol::qualified("sim", "openai-gateway"));
    assert!(manifest.capabilities.is_empty());
    assert_eq!(manifest.exports.len(), 19);
    assert!(
        manifest
            .exports
            .iter()
            .any(|export| export.symbol() == &openai_codec_symbol())
    );
}

#[test]
fn serve_capability_symbol_is_stable() {
    let capability = openai_gateway_serve_capability();

    assert_eq!(capability.as_str(), OPENAI_GATEWAY_SERVE_CAPABILITY);
    assert_eq!(
        capability.as_symbol(),
        Symbol::qualified("capability", "openai-gateway.serve")
    );
}

#[test]
fn health_handler_returns_json_without_network() {
    let request = GatewayRequest::get(HEALTH_PATH);
    let response = configure_routes().handle(&request);

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(response.body(), HEALTH_BODY.as_bytes());
}

#[test]
fn health_response_construction_is_separate_from_route_registration() {
    let response = health_response();

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(response.body(), HEALTH_BODY.as_bytes());
}

#[test]
fn route_registration_handles_missing_and_wrong_method() {
    let routes = configure_routes();

    let missing = routes.handle(&GatewayRequest::get("/missing"));
    assert_eq!(missing.status(), 404);
    let wrong_method = routes.handle(&GatewayRequest::new(
        "POST",
        HEALTH_PATH,
        Vec::new(),
        Vec::new(),
    ));
    assert_eq!(wrong_method.status(), 405);
}

#[test]
fn models_handler_returns_fixture_and_sim_native_models_without_network() {
    let request = GatewayRequest::get(MODELS_PATH);
    let response = configure_routes().handle(&request);
    let json: Value = serde_json::from_slice(response.body()).unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));
    assert_eq!(json["object"], "list");
    assert_eq!(json["data"][0]["id"], "fixture/echo");
    assert!(
        json["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "sim/browse/plan")
    );
}

#[test]
fn model_catalog_converts_runner_cards_after_fixtures() {
    let catalog = ModelCatalog::from_runner_card_exprs(vec![model_card_expr(
        Symbol::new("runner/stub"),
        "stub/model",
        Symbol::new("stub-provider"),
        Symbol::new("local"),
    )])
    .unwrap();
    let response = models_response_for_catalog(&catalog);
    let json: Value = serde_json::from_slice(response.body()).unwrap();

    assert_eq!(json["data"][0]["id"], "fixture/echo");
    assert_eq!(json["data"][1]["id"], "stub/model");
    assert_eq!(json["data"][1]["owned_by"], "stub-provider");
    assert_eq!(json["data"][2]["id"], "sim/browse/plan");
}

#[test]
fn gateway_lib_registers_health_and_serve_functions() {
    let mut cx = cx();

    install_openai_gateway_lib(&mut cx).unwrap();

    assert!(cx.resolve_function(&health_symbol()).is_ok());
    assert!(cx.resolve_function(&serve_symbol()).is_ok());
    assert!(cx.resolve_function(&models_symbol()).is_ok());
    assert!(cx.resolve_function(&plan_parse_symbol()).is_ok());
    assert!(cx.resolve_function(&plan_check_symbol()).is_ok());
    assert!(cx.resolve_function(&plan_run_symbol()).is_ok());
    assert!(cx.resolve_function(&plan_explain_symbol()).is_ok());
    assert!(cx.resolve_function(&plan_combinators_symbol()).is_ok());
    assert!(cx.resolve_function(&fabric_symbol()).is_ok());
    assert!(cx.resolve_codec(&openai_codec_symbol()).is_ok());
    assert!(cx.registry().codecs().contains_key(&openai_codec_symbol()));
}

#[test]
fn models_function_collects_runner_cards_via_runner_cards() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    install_runner_cards_stub(&mut cx);

    let runner = cx.factory().string("runner-value".to_owned()).unwrap();
    let value = OpenAiGatewayFunction::models()
        .call(&mut cx, Args::new(vec![runner]))
        .unwrap();
    let response = value
        .object()
        .downcast_ref::<GatewayResponseValue>()
        .unwrap()
        .response();
    let json: Value = serde_json::from_slice(response.body()).unwrap();

    assert_eq!(json["data"][0]["id"], "fixture/echo");
    assert!(
        json["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "stub/runner-card")
    );
}

#[test]
fn health_function_returns_gateway_response_value() {
    let mut cx = cx();

    let value = OpenAiGatewayFunction::health()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap();
    let response = value
        .object()
        .downcast_ref::<GatewayResponseValue>()
        .unwrap()
        .response();

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("application/json"));

    let extra = cx.factory().string("unexpected".to_owned()).unwrap();
    let err = OpenAiGatewayFunction::health()
        .call(&mut cx, sim_kernel::Args::new(vec![extra]))
        .unwrap_err();
    assert!(matches!(err, Error::Eval(message) if message.contains("expects no arguments")));
}

#[test]
fn serve_function_requires_gateway_network_and_webhook_capabilities() {
    let mut cx = cx();

    let err = OpenAiGatewayFunction::serve()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == openai_gateway_serve_capability()
    ));

    cx.grant(openai_gateway_serve_capability());
    let err = OpenAiGatewayFunction::serve()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == network_capability()
    ));

    cx.grant(CapabilityName::new("network"));
    let err = OpenAiGatewayFunction::serve()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == webhook_serve_capability()
    ));

    cx.grant(webhook_serve_capability());
    let value = OpenAiGatewayFunction::serve()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap();
    let routes = value
        .object()
        .downcast_ref::<GatewayRoutesValue>()
        .unwrap()
        .routes();
    let paths = routes.paths();
    assert_eq!(paths.len(), 30);
    for path in [
        HEALTH_PATH,
        BATCHES_PATH,
        FILES_PATH,
        THREAD_MESSAGES_ROUTE,
        RESPONSE_RETRIEVAL_ROUTE,
        ADMIN_STORAGE_STATS_PATH,
    ] {
        assert!(paths.contains(&path));
    }
}

#[test]
fn openai_codec_decodes_minimal_chat_request() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();

    let decoded = decode_with_codec(
        &mut cx,
        &openai_codec_symbol(),
        Input::Text(
            r#"{"model":"gpt-4.1-mini","messages":[{"role":"user","content":"hello"}]}"#.to_owned(),
        ),
        ReadPolicy::default(),
    )
    .unwrap();

    validate_chat_transcript(&decoded).unwrap();
    assert!(format!("{decoded:?}").contains("model-request"));
    assert!(format!("{decoded:?}").contains("hello"));
}

#[test]
fn openai_codec_encodes_model_response_json() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    let response = model_response_expr(
        Symbol::new("fixture"),
        "gpt-4.1-mini",
        vec![content_part("pong")],
        Symbol::new("stop"),
    );

    let output = encode_with_codec(
        &mut cx,
        &openai_codec_symbol(),
        &response,
        EncodeOptions::default(),
    )
    .unwrap();
    let text = match output {
        Output::Text(text) => text,
        Output::Bytes(bytes) => String::from_utf8(bytes).unwrap(),
    };
    let json: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(json["object"], "chat.completion");
    assert_eq!(json["model"], "gpt-4.1-mini");
    assert_eq!(json["choices"][0]["message"]["role"], "assistant");
    assert_eq!(json["choices"][0]["message"]["content"], "pong");
}

#[test]
fn openai_decode_roundtrips_through_chat_codec() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();
    let chat = ChatCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&chat).unwrap();

    let decoded = decode_with_codec(
        &mut cx,
        &openai_codec_symbol(),
        Input::Text(
            r#"{"model":"gpt-4.1-mini","messages":[{"role":"system","content":[{"type":"text","text":"brief"}]},{"role":"user","content":[{"type":"text","text":"summarize"}]}],"stream":false}"#
                .to_owned(),
        ),
        ReadPolicy::default(),
    )
    .unwrap();
    let encoded = encode_with_codec(
        &mut cx,
        &Symbol::qualified("codec", "chat"),
        &decoded,
        EncodeOptions::default(),
    )
    .unwrap();
    let input = match encoded {
        Output::Text(text) => Input::Text(text),
        Output::Bytes(bytes) => Input::Bytes(bytes),
    };
    let chat_decoded = decode_with_codec(
        &mut cx,
        &Symbol::qualified("codec", "chat"),
        input,
        ReadPolicy::default(),
    )
    .unwrap();

    assert!(chat_decoded.canonical_eq(&decoded));
}

#[test]
fn openai_codec_rejects_out_of_domain_content_parts() {
    let mut cx = cx();
    install_openai_gateway_lib(&mut cx).unwrap();

    let err = decode_with_codec(
        &mut cx,
        &openai_codec_symbol(),
        Input::Text(
            r#"{"model":"gpt-4.1-mini","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"x"}}]}]}"#
                .to_owned(),
        ),
        ReadPolicy::default(),
    )
    .unwrap_err();

    assert!(matches!(err, Error::CodecError { .. }));
}

#[test]
fn openai_provider_request_encoder_matches_fixture_shape() {
    let body = encode_openai_request(
        &request_expr(),
        &OpenAiRequestOptions::new("gpt-4.1-mini", false, true),
    )
    .unwrap();
    let text = String::from_utf8(body).unwrap();

    assert!(text.contains("\"model\":\"gpt-4.1-mini\""));
    assert!(text.contains("\"stream\":false"));
    assert!(text.contains("\"role\":\"system\""));
    assert!(text.contains("\"summarize this file\""));
}

#[test]
fn openai_provider_response_decoder_matches_fixture_shape() {
    let expr = decode_openai_response(
        Symbol::new("remote"),
        "gpt-4.1-mini",
        br#"{"id":"chatcmpl-1","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"compiled"}}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#,
        true,
    )
    .unwrap();

    validate_chat_transcript(&expr).unwrap();
    assert!(format!("{expr:?}").contains("compiled"));
    assert!(format!("{expr:?}").contains("raw-provider-response"));
    assert!(format!("{expr:?}").contains("input-tokens"));
}

fn request_expr() -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("summarize this file".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(vec![message_expr("system", "Answer in precise prose.")]),
        ),
    ])
}

fn message_expr(role: &str, text: &str) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("role")),
            Expr::Symbol(Symbol::new(role.to_owned())),
        ),
        (
            Expr::Symbol(Symbol::new("content")),
            Expr::List(vec![content_part(text)]),
        ),
    ])
}

fn content_part(text: &str) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("type")),
            Expr::Symbol(Symbol::new("text")),
        ),
        (
            Expr::Symbol(Symbol::new("text")),
            Expr::String(text.to_owned()),
        ),
    ])
}

#[derive(Clone)]
struct RunnerCardsStub;

impl Object for RunnerCardsStub {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<runner-cards-stub>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for RunnerCardsStub {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for RunnerCardsStub {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<sim_kernel::Value> {
        if args.values().is_empty() {
            return Err(Error::Eval("runner/cards stub expects runners".to_owned()));
        }
        cx.factory().expr(Expr::List(vec![model_card_expr(
            Symbol::new("runner/stub"),
            "stub/runner-card",
            Symbol::new("stub-provider"),
            Symbol::new("local"),
        )]))
    }
}

fn install_runner_cards_stub(cx: &mut Cx) {
    let value = cx.factory().opaque(Arc::new(RunnerCardsStub)).unwrap();
    cx.registry_mut()
        .register_function_value(Symbol::qualified("runner", "cards"), value)
        .unwrap();
}
