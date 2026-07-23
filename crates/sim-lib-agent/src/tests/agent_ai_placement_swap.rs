use super::support::{eval_cx, flatten_text, install_agent_lib, install_test_codec};
use crate::{AI_RUNNER_PLACEMENT_CAPABILITY, components::cached_model_fabric_value};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{
    AbiVersion, Args, Callable, Consistency, EvalMode, EvalReply, EvalRequest, Export, Expr, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_lib_stream_fabric::{ContentKey, EvalCassette, EvalCassetteLedger};
use sim_value::access::field as map_field;
use std::sync::{Arc, Mutex};

// conformance: model runner framework realizes one prompt through open placements.

#[derive(Default)]
struct MemoryLedger {
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EvalCassetteLedger for MemoryLedger {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>> {
        Ok(self.entries.lock().unwrap().clone())
    }
}

#[derive(Clone)]
struct StubLoadedModelSite {
    runner: String,
    model: String,
    answer_prefix: String,
    calls: Option<Arc<Mutex<usize>>>,
}

impl Object for StubLoadedModelSite {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> Result<String> {
        Ok(format!("#<stub-model-site {}>", self.model))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for StubLoadedModelSite {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for StubLoadedModelSite {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> Result<Value> {
        let [request] = args.values() else {
            return Err(sim_kernel::Error::Eval(
                "stub model site expects one request argument".to_owned(),
            ));
        };
        if let Some(calls) = &self.calls {
            *calls.lock().unwrap() += 1;
        }
        let request = request.object().as_expr(cx)?;
        let expr = map_field(&request, "expr").ok_or_else(|| {
            sim_kernel::Error::Eval("stub model site request missing expr".to_owned())
        })?;
        let task = match map_field(expr, "task") {
            Some(Expr::String(task)) => task.as_str(),
            _ => "request",
        };
        let response = ModelResponse::new(
            Symbol::new(self.runner.clone()),
            self.model.clone(),
            vec![text_content(format!("{}: {task}", self.answer_prefix))],
            Symbol::new("stop"),
        );
        cx.factory().expr(response.into())
    }
}

struct StubLoadedModelSiteLib {
    symbol: Symbol,
    site: Value,
}

impl Lib for StubLoadedModelSiteLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("test", format!("placement-swap-{}", self.symbol.name)),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Site {
                symbol: self.symbol.clone(),
                runtime_id: None,
            }],
        }
    }

    fn load(&self, _cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.site_value(self.symbol.clone(), self.site.clone())?;
        Ok(())
    }
}

fn model_request(task: &str) -> EvalRequest {
    EvalRequest {
        expr: model_request_expr(task),
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

fn model_request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        key_expr("model-request", Expr::Bool(true)),
        key_expr("task", Expr::String(task.to_owned())),
        key_expr("messages", Expr::List(Vec::new())),
    ])
}

fn fake_runner(cx: &mut sim_kernel::Cx, text: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            cx.factory()
                .expr(Expr::List(vec![Expr::String(text.to_owned())]))
                .unwrap(),
        ]),
    )
    .unwrap()
}

fn mock_http_runner(cx: &mut sim_kernel::Cx) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "echo"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory()
                .symbol(Symbol::new("mock-http-runner"))
                .unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string("remote/mock-http".to_owned()).unwrap(),
        ]),
    )
    .unwrap()
}

fn place_runner(cx: &mut sim_kernel::Cx, key: &str, runner: Value) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    cx.call_function(
        &Symbol::qualified("runner", "place"),
        Args::new(vec![cx.factory().string(key.to_owned()).unwrap(), runner]),
    )
    .unwrap();
}

fn model_at(cx: &mut sim_kernel::Cx, key: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("model", "at"),
        Args::new(vec![cx.factory().string(key.to_owned()).unwrap()]),
    )
    .unwrap()
}

fn cached_with_ledger(cx: &mut sim_kernel::Cx, fabric: Value, ledger: Arc<MemoryLedger>) -> Value {
    let cassette = Arc::new(EvalCassette::new(ledger));
    cached_model_fabric_value(cx, fabric, cassette).unwrap()
}

fn register_loaded_model_site(
    cx: &mut sim_kernel::Cx,
    key: &str,
    model: &str,
    answer_prefix: &str,
    calls: Option<Arc<Mutex<usize>>>,
) {
    let site = cx
        .factory()
        .opaque(Arc::new(StubLoadedModelSite {
            runner: key.replace(':', "/"),
            model: model.to_owned(),
            answer_prefix: answer_prefix.to_owned(),
            calls,
        }))
        .unwrap();
    cx.load_lib(&StubLoadedModelSiteLib {
        symbol: Symbol::new(key.to_owned()),
        site,
    })
    .unwrap();
}

fn realize_on(cx: &mut sim_kernel::Cx, fabric: Value, request: EvalRequest) -> Expr {
    let reply = fabric
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(cx, request)
        .unwrap();
    reply.value.object().as_expr(cx).unwrap()
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        key_expr("type", Expr::Symbol(Symbol::new("text"))),
        key_expr("text", Expr::String(text)),
    ])
}

#[test]
fn one_prompt_graph_realizes_against_three_placements() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let fake = fake_runner(&mut cx, "ok-fake");
    place_runner(&mut cx, "model-site:fake", fake);
    register_loaded_model_site(
        &mut cx,
        "model-site:local",
        "sim-local-modeled",
        "ok-local",
        None,
    );
    let remote = mock_http_runner(&mut cx);
    place_runner(&mut cx, "model-site:remote", remote);

    let request = model_request("summarize src/lib.rs");
    let original_expr = request.expr.clone();
    let cases = [
        ("model-site:fake", "ok-fake"),
        ("model-site:local", "ok-local: summarize src/lib.rs"),
        ("model-site:remote", "summarize src/lib.rs"),
    ];
    for (key, expected_text) in cases {
        let placement = model_at(&mut cx, key);
        let reply = realize_on(&mut cx, placement, request.clone());
        validate_chat_transcript(&reply).unwrap();
        ModelResponse::try_from(reply.clone()).unwrap();
        assert_eq!(request.expr, original_expr);
        assert!(
            flatten_text(&reply).contains(expected_text),
            "{key} reply did not include {expected_text:?}: {reply:?}"
        );
    }
}

#[test]
fn cached_local_placement_skips_second_inference() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let calls = Arc::new(Mutex::new(0usize));
    register_loaded_model_site(
        &mut cx,
        "model-site:cached-local",
        "sim-local-modeled",
        "cached-local",
        Some(calls.clone()),
    );
    let ledger = Arc::new(MemoryLedger::default());
    let placement = model_at(&mut cx, "model-site:cached-local");
    let cached = cached_with_ledger(&mut cx, placement, ledger);

    let first = realize_on(&mut cx, cached.clone(), model_request("same local prompt"));
    let second = realize_on(&mut cx, cached, model_request("same local prompt"));

    validate_chat_transcript(&first).unwrap();
    validate_chat_transcript(&second).unwrap();
    assert_eq!(map_field(&first, "cache-hit"), Some(&Expr::Bool(false)));
    assert_eq!(map_field(&second, "cache-hit"), Some(&Expr::Bool(true)));
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(flatten_text(&second).contains("cached-local: same local prompt"));
}
