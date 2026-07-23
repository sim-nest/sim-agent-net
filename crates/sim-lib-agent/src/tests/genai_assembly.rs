use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

// conformance: the GenAI assembly recipe runs one checked ASK exchange through a placed target.

use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use crate::{
    AI_RUNNER_PLACEMENT_CAPABILITY, ExternalRunnerSpec, ModelCard, ModelRequest, ModelResponse,
    ModelRunner, RECIPES, external_runner_value,
};
use sim_codec::{Input, decode_eval_expr_with_codec, decode_with_codec, lower_operator_nodes};
use sim_cookbook::{RecipeCard, RecipeStore};
use sim_kernel::{
    Args, CapabilityName, CapabilitySet, Cx, EvalRequest, Expr, ReadPolicy, Result, Symbol, Value,
    macro_expand_eval_capability, read_construct_capability, read_eval_capability,
};
use sim_lib_core::{SurfacePackLib, SurfacePackSpec};
use sim_value::{access::field, build::entry};

const RECIPE_ID: &str = "agent/01-basics/genai-assembly";
const MODEL_SITE: &str = "model-site:genai";
const SETUP_SOURCE: &str = include_str!("../../recipes/01-basics/genai-assembly/setup.siml");

struct RecordingRunner {
    requests: Mutex<Vec<Expr>>,
    responses: Mutex<VecDeque<String>>,
}

impl RecordingRunner {
    fn new(responses: impl IntoIterator<Item = String>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    fn requests(&self) -> Vec<Expr> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelRunner for RecordingRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            Symbol::qualified("runner", "genai-recording"),
            "genai/recording",
            Symbol::new("fixture"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<ModelResponse> {
        let text = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| sim_kernel::Error::Eval("recording runner exhausted".to_owned()))?;
        Ok(model_response("genai/recording", text))
    }

    fn infer_request(&self, cx: &mut Cx, request: EvalRequest) -> Result<ModelResponse> {
        self.requests.lock().unwrap().push(request.expr);
        self.infer(cx, ModelRequest::default())
    }
}

#[test]
fn genai_assembly_recipe_runs_exact_source_against_replaceable_target() {
    let mut cx = recipe_cx();
    let card = genai_card();
    assert_eq!(setup_text(&card), SETUP_SOURCE);

    let recording = Arc::new(RecordingRunner::new([json_text(&Expr::String(
        "recorded checked answer".to_owned(),
    ))]));
    let runner = external_runner_value(
        &mut cx,
        ExternalRunnerSpec {
            symbol: Symbol::qualified("runner", "genai-recording"),
            model: "genai/recording".to_owned(),
            capabilities: Vec::new(),
            spec: Vec::new(),
            runner: recording.clone(),
        },
    )
    .unwrap();
    place_runner(&mut cx, runner, false);

    let run = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &run);
    assert_eq!(
        reply_payload(&mut cx, &run),
        Expr::String("recorded checked answer".to_owned())
    );
    assert_recorded_contract(&recording.requests());

    let fake = fake_runner(
        &mut cx,
        "genai-repair-fake",
        "genai/fake",
        [
            json_text(&Expr::Bool(false)),
            json_text(&Expr::String("fake repaired answer".to_owned())),
        ],
    );
    place_runner(&mut cx, fake, true);
    let repaired = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &repaired);
    assert_eq!(
        reply_payload(&mut cx, &repaired),
        Expr::String("fake repaired answer".to_owned())
    );

    let replacement = fake_runner(
        &mut cx,
        "genai-replacement-fake",
        "genai/replacement",
        [json_text(&Expr::String("replacement answer".to_owned()))],
    );
    place_runner(&mut cx, replacement, true);
    let replaced = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &replaced);
    assert_eq!(
        reply_payload(&mut cx, &replaced),
        Expr::String("replacement answer".to_owned())
    );

    assert_eq!(setup_text(&card), SETUP_SOURCE);
}

fn recipe_cx() -> Cx {
    let mut cx = eval_cx();
    let core = SurfacePackLib {
        spec: SurfacePackSpec {
            lib_id: sim_lib_core::manifest_name(),
            values: Vec::new(),
        },
    };
    cx.load_lib(&core).unwrap();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    sim_lib_bridge::install_bridge_lib(&mut cx).unwrap();
    cx.grant(read_eval_capability());
    cx.grant(macro_expand_eval_capability());
    cx.grant(CapabilityName::new("ai/run"));
    cx
}

fn genai_card() -> RecipeCard {
    let mut store = RecipeStore::new();
    store.register_book(RECIPES).unwrap();
    store.card(RECIPE_ID).cloned().unwrap()
}

fn assert_run_ok(cx: &mut Cx, card: &RecipeCard, run: &sim_cookbook::RecipeRun) {
    if run.ok {
        return;
    }
    let expr = lower_operator_nodes(
        decode_eval_expr_with_codec(
            cx,
            &Symbol::qualified("codec", card.codec.as_str()),
            Input::Text(setup_text(card).to_owned()),
            trusted_recipe_read_policy(),
        )
        .unwrap(),
    );
    let allowed = sim_lib_cookbook::CookbookCapabilityProfile::granted()
        .into_iter()
        .fold(CapabilitySet::new(), CapabilitySet::grant)
        .grant(read_construct_capability());
    let direct = cx.with_capabilities(allowed, |cx| cx.eval_expr(expr));
    panic!("expected recipe run to pass, got {run:?}; direct eval: {direct:?}");
}

fn trusted_recipe_read_policy() -> ReadPolicy {
    ReadPolicy {
        trust: sim_kernel::TrustLevel::TrustedSource,
        capabilities: CapabilitySet::new()
            .grant(read_construct_capability())
            .grant(read_eval_capability())
            .grant(macro_expand_eval_capability()),
    }
}

fn fake_runner(
    cx: &mut Cx,
    name: &str,
    model: &str,
    script: impl IntoIterator<Item = String>,
) -> Value {
    let script_value = cx
        .factory()
        .expr(Expr::List(script.into_iter().map(Expr::String).collect()))
        .unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn place_runner(cx: &mut Cx, runner: Value, replace: bool) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    let mut args = vec![
        cx.factory().string(MODEL_SITE.to_owned()).unwrap(),
        runner.clone(),
    ];
    if replace {
        args.push(cx.factory().symbol(Symbol::new(":replace")).unwrap());
        args.push(cx.factory().bool(true).unwrap());
    }
    cx.call_function(&Symbol::qualified("runner", "place"), Args::new(args))
        .unwrap();
}

fn model_response(model: &str, text: String) -> ModelResponse {
    let expr = sim_codec_chat::model_response_expr(
        Symbol::qualified("runner", "genai-recording"),
        model,
        vec![text_content(text)],
        Symbol::new("stop"),
    );
    ModelResponse::try_from(expr).unwrap()
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn json_text(expr: &Expr) -> String {
    sim_codec_json::expr_to_json(expr).to_string()
}

fn setup_text(card: &RecipeCard) -> &str {
    std::str::from_utf8(&card.setup).unwrap()
}

fn reply_payload(cx: &mut Cx, run: &sim_cookbook::RecipeRun) -> Expr {
    assert_eq!(run.forms, 1);
    let expr = decode_lisp(cx, &run.results[0]);
    let Some(Expr::Vector(parts)) = field(&expr, "body") else {
        panic!("reply packet missing body: {expr:?}");
    };
    let Some(part) = parts.first() else {
        panic!("reply packet has empty body: {expr:?}");
    };
    field(part, "payload").cloned().unwrap()
}

fn decode_lisp(cx: &mut Cx, text: &str) -> Expr {
    decode_with_codec(
        cx,
        &Symbol::qualified("codec", "lisp"),
        Input::Text(text.to_owned()),
        ReadPolicy::default(),
    )
    .unwrap()
}

fn assert_recorded_contract(requests: &[Expr]) {
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let Some(Expr::String(face)) = field(request, "task") else {
        panic!("model task was not rendered text: {request:?}");
    };
    assert!(face.contains("CALL-DATA"), "{face}");
    assert!(face.contains("Explain SIM in one sentence."), "{face}");
    assert_eq!(
        field(request, "return-codec"),
        Some(&Expr::Symbol(Symbol::qualified("codec", "json")))
    );
    assert_eq!(
        field(request, "return-shape"),
        Some(&Expr::Symbol(Symbol::qualified("core", "String")))
    );

    let Some(Expr::Vector(calls)) = field(request, "bridge-calls") else {
        panic!("model request missing bridge-calls: {request:?}");
    };
    let Some(call) = calls.first() else {
        panic!("bridge-calls was empty");
    };
    assert_eq!(
        field(call, "name"),
        Some(&Expr::Symbol(Symbol::qualified("genai", "generate")))
    );
    let Some(model_params) = field(call, "model-params") else {
        panic!("bridge call missing model params: {call:?}");
    };
    assert_eq!(
        field(model_params, "temperature"),
        Some(&Expr::String("0".to_owned()))
    );
}
