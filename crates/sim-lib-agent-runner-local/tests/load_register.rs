use std::sync::Arc;

// conformance: EvalFabric site registration exposes modeled local placement.

use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{
    Args, Consistency, Cx, DefaultFactory, EagerPolicy, EvalMode, EvalRequest, Export, Expr, Lib,
    Symbol,
};
use sim_lib_agent_runner_core::{ModelCard, ModelResponse, ModelRunner};
use sim_lib_agent_runner_local::{
    LOCAL_MODEL_SITE_KEY, LocalModelBackend, LocalModelLib, local_model_site_symbol,
};

fn eval_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

fn install_binary_codec(cx: &mut Cx) {
    let binary = sim_codec_binary::BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
}

fn model_request(task: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::Map(vec![
            (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("task")),
                Expr::String(task.to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("messages")),
                Expr::List(Vec::new()),
            ),
        ]),
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

#[test]
fn host_lib_registers_local_model_site() {
    let mut cx = eval_cx();
    install_binary_codec(&mut cx);
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();

    cx.load_lib(&LocalModelLib).unwrap();

    let symbol = local_model_site_symbol();
    assert!(cx.registry().site_by_symbol(&symbol).is_some());
    assert!(LocalModelLib.manifest().exports.iter().any(|export| {
        matches!(export, Export::Site { symbol: exported, .. } if exported == &symbol)
    }));
}

#[test]
fn model_at_realizes_loaded_local_stub() {
    let mut cx = eval_cx();
    install_binary_codec(&mut cx);
    sim_lib_agent::install_agent_lib(&mut cx).unwrap();
    cx.load_lib(&LocalModelLib).unwrap();

    let key = cx
        .factory()
        .string(LOCAL_MODEL_SITE_KEY.to_owned())
        .unwrap();
    let placement = cx
        .call_function(&Symbol::qualified("model", "at"), Args::new(vec![key]))
        .unwrap();
    let reply = placement
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("local prompt"))
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();

    validate_chat_transcript(&expr).unwrap();
    let response = ModelResponse::try_from(expr).unwrap();
    assert_eq!(response.runner, Symbol::new("runner/local-model"));
    assert_eq!(response.model, "sim-local-modeled");
    assert!(format!("{:?}", response.content).contains("sim-local-modeled-ok: local prompt"));
}

#[test]
fn card_reports_local_placement_and_capabilities() {
    let card = LocalModelBackend::new().card();
    assert_eq!(card.locality, Symbol::new("local"));
    assert_eq!(card.model, "sim-local-modeled");
    assert_eq!(
        find_extra_string(&card, "placement-key"),
        Some(LOCAL_MODEL_SITE_KEY)
    );
    assert_eq!(find_extra_bool(&card, "supports-stream"), Some(true));
    assert_eq!(find_extra_bool(&card, "supports-cache"), Some(true));
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

fn find_extra_bool(card: &ModelCard, key: &str) -> Option<bool> {
    card.extra
        .iter()
        .find_map(|(entry_key, value)| match (entry_key, value) {
            (Expr::Symbol(symbol), Expr::Bool(value)) if symbol.name.as_ref() == key => {
                Some(*value)
            }
            _ => None,
        })
}
