#![cfg(feature = "wasm-model")]

use std::sync::Arc;

use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{
    Args, Consistency, Cx, DefaultFactory, EagerPolicy, Error, EvalMode, EvalRequest, Expr, Symbol,
};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_lib_agent_runner_local::{
    LOCAL_WASM_MODEL_RUNNER, LOCAL_WASM_MODEL_SITE_KEY, WasmModelLib, WasmModelLimits,
    ai_runner_local_capability, ai_runner_wasm_capability, load_wasm_model,
    local_wasm_model_site_symbol,
};
use sim_wasm_abi::{
    Frame,
    model::{WasmModelFrameRef, encode_model_expr_frame, pack_model_frame_ref},
};

fn eval_cx() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x3c55_425a_b30e_cc05),
    )
}

fn grant_wasm_model_caps(cx: &mut Cx) {
    cx.grant(ai_runner_local_capability());
    cx.grant(ai_runner_wasm_capability());
}

fn install_binary_codec(cx: &mut Cx) {
    let binary = sim_codec_binary::BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
}

fn eval_model_request(task: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::from(model_request(task)),
        result_shape: None,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::LocalFirst,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn model_request(task: &str) -> ModelRequest {
    ModelRequest::new(Expr::String(task.to_owned()), Vec::new())
}

fn model_card() -> ModelCard {
    let mut card = ModelCard::new(
        Symbol::new(LOCAL_WASM_MODEL_RUNNER),
        "sim-wasm-echo",
        Symbol::new("wasm-model"),
        Symbol::new("local"),
    );
    card.extra.push(key_expr(
        "placement-key",
        Expr::String(LOCAL_WASM_MODEL_SITE_KEY.to_owned()),
    ));
    card
}

fn model_response() -> ModelResponse {
    ModelResponse::new(
        Symbol::new(LOCAL_WASM_MODEL_RUNNER),
        "sim-wasm-echo",
        vec![Expr::Map(vec![
            key_expr("type", Expr::Symbol(Symbol::new("text"))),
            key_expr("text", Expr::String("wasm echo model ok".to_owned())),
        ])],
        Symbol::new("stop"),
    )
}

#[test]
fn wasm_model_load_denied_without_capability() {
    let cx = eval_cx();
    let request = model_request("capability check");
    let wasm = echo_model_wasm(&request, &model_response());

    let error = match load_wasm_model(&cx, &wasm, WasmModelLimits::default()) {
        Ok(_) => panic!("wasm model loaded without required capability"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        Error::CapabilityDenied { capability } if capability == ai_runner_local_capability()
    ));
}

#[test]
fn wasm_model_load_allowed_with_capability() {
    let mut cx = eval_cx();
    grant_wasm_model_caps(&mut cx);
    let request = model_request("allowed");
    let wasm = echo_model_wasm(&request, &model_response());

    let backend = load_wasm_model(&cx, &wasm, WasmModelLimits::default()).unwrap();
    let card = backend.card();
    assert_eq!(card.runner, Symbol::new(LOCAL_WASM_MODEL_RUNNER));
    assert_eq!(card.model, "sim-wasm-echo");
    assert_eq!(card.locality, Symbol::new("local"));
    assert_eq!(
        find_extra_string(&card, "placement-key"),
        Some(LOCAL_WASM_MODEL_SITE_KEY)
    );
}

#[test]
fn wasm_model_fuel_exhaustion_returns_error() {
    let mut cx = eval_cx();
    grant_wasm_model_caps(&mut cx);
    let wasm = fuel_loop_model_wasm();
    let backend = load_wasm_model(
        &cx,
        &wasm,
        WasmModelLimits {
            fuel_per_infer: 1_000,
            max_memory_pages: 1,
        },
    )
    .unwrap();

    let error = backend.infer(&mut cx, model_request("loop")).unwrap_err();
    assert!(
        matches!(error, Error::Eval(message) if message.contains("wasm model inference failed"))
    );
}

#[test]
fn wasm_model_echo_fixture_round_trips_request_to_model_response() {
    let mut cx = eval_cx();
    grant_wasm_model_caps(&mut cx);
    let request = model_request("echo");
    let expected = model_response();
    let wasm = echo_model_wasm(&request, &expected);

    let backend = load_wasm_model(&cx, &wasm, WasmModelLimits::default()).unwrap();
    let response = backend.infer(&mut cx, request).unwrap();
    let response_expr: Expr = response.clone().into();

    validate_chat_transcript(&response_expr).unwrap();
    assert_eq!(response, expected);
}

#[test]
fn wasm_model_lib_registers_local_wasm_site() {
    let mut cx = eval_cx();
    install_binary_codec(&mut cx);
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();
    grant_wasm_model_caps(&mut cx);
    let eval_request = eval_model_request("site echo");
    let request = ModelRequest::try_from(eval_request.expr.clone()).unwrap();
    let wasm = echo_model_wasm(&request, &model_response());
    let backend = load_wasm_model(&cx, &wasm, WasmModelLimits::default()).unwrap();

    cx.load_lib(&WasmModelLib::new(backend)).unwrap();

    let symbol = local_wasm_model_site_symbol();
    assert!(cx.registry().site_by_symbol(&symbol).is_some());
    let key = cx
        .factory()
        .string(LOCAL_WASM_MODEL_SITE_KEY.to_owned())
        .unwrap();
    let placement = cx
        .call_function(&Symbol::qualified("model", "at"), Args::new(vec![key]))
        .unwrap();
    let reply = placement
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, eval_request)
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();

    validate_chat_transcript(&expr).unwrap();
    let response = ModelResponse::try_from(expr).unwrap();
    assert_eq!(response.runner, Symbol::new(LOCAL_WASM_MODEL_RUNNER));
    assert_eq!(response.model, "sim-wasm-echo");
}

fn echo_model_wasm(request: &ModelRequest, response: &ModelResponse) -> Vec<u8> {
    let card_frame = encode_model_expr_frame(&Expr::from(model_card())).unwrap();
    let request_frame = encode_model_expr_frame(&Expr::from(request.clone())).unwrap();
    let response_frame = encode_model_expr_frame(&Expr::from(response.clone())).unwrap();
    wat::parse_str(echo_model_wat(&card_frame, &request_frame, &response_frame)).unwrap()
}

fn echo_model_wat(card_frame: &Frame, request_frame: &Frame, response_frame: &Frame) -> String {
    include_str!("../src/fixtures/echo_model.wat")
        .replace("__CARD_FRAME__", &wat_bytes(card_frame.bytes()))
        .replace("__RESPONSE_FRAME__", &wat_bytes(response_frame.bytes()))
        .replace("__REQUEST_FRAME__", &wat_bytes(request_frame.bytes()))
        .replace("__CARD_REF__", &packed_ref(0, card_frame).to_string())
        .replace(
            "__RESPONSE_REF__",
            &packed_ref(2048, response_frame).to_string(),
        )
        .replace("__REQUEST_LEN__", &request_frame.bytes().len().to_string())
}

fn fuel_loop_model_wasm() -> Vec<u8> {
    let card_frame = encode_model_expr_frame(&Expr::from(model_card())).unwrap();
    let wat = format!(
        r#"(module
          (memory (export "memory") 1)
          (data (i32.const 0) "{}")
          (func (export "sim_alloc") (param $len i32) (result i32)
            (i32.const 1024))
          (func (export "sim_model_card") (result i64)
            (i64.const {}))
          (func (export "sim_model_infer") (param $ptr i32) (param $len i32) (result i64)
            (loop $again
              (br $again))
            (i64.const 0)))"#,
        wat_bytes(card_frame.bytes()),
        packed_ref(0, &card_frame)
    );
    wat::parse_str(wat).unwrap()
}

fn packed_ref(ptr: u32, frame: &Frame) -> u64 {
    pack_model_frame_ref(WasmModelFrameRef {
        ptr,
        len: frame.bytes().len().try_into().unwrap(),
    })
}

fn wat_bytes(bytes: &[u8]) -> String {
    let mut encoded = String::new();
    for byte in bytes {
        encoded.push_str(&format!("\\{byte:02x}"));
    }
    encoded
}

fn find_extra_string<'a>(card: &'a ModelCard, key: &str) -> Option<&'a str> {
    card.extra
        .iter()
        .find_map(|(entry_key, value)| match (entry_key, value) {
            (Expr::Symbol(symbol), Expr::String(text)) if symbol.name.as_ref() == key => {
                Some(text.as_str())
            }
            _ => None,
        })
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}
