use super::support::{eval_cx, flatten_text, install_agent_lib, install_test_codec};
use crate::{AI_RUNNER_PLACEMENT_CAPABILITY, components::cached_model_fabric_value};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Args, Consistency, EvalMode, EvalReply, EvalRequest, Expr, Symbol, Value};
use sim_lib_stream_fabric::{ContentKey, EvalCassette, EvalCassetteLedger};
use sim_value::access::field as map_field;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MemoryLedger {
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EvalCassetteLedger for MemoryLedger {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> sim_kernel::Result<()> {
        self.entries
            .lock()
            .unwrap()
            .push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> sim_kernel::Result<Vec<(ContentKey, EvalReply)>> {
        Ok(self.entries.lock().unwrap().clone())
    }
}

fn model_request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
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

fn fake_runner(cx: &mut sim_kernel::Cx, answer: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            cx.factory()
                .expr(Expr::List(vec![Expr::String(answer.to_owned())]))
                .unwrap(),
        ]),
    )
    .unwrap()
}

fn model_at(cx: &mut sim_kernel::Cx, key: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("model", "at"),
        Args::new(vec![cx.factory().string(key.to_owned()).unwrap()]),
    )
    .unwrap()
}

fn model_cached(cx: &mut sim_kernel::Cx, fabric: Value) -> Value {
    cx.call_function(
        &Symbol::qualified("model", "cached"),
        Args::new(vec![fabric]),
    )
    .unwrap()
}

fn cached_with_ledger(cx: &mut sim_kernel::Cx, fabric: Value, ledger: Arc<MemoryLedger>) -> Value {
    let cassette = Arc::new(EvalCassette::new(ledger));
    cached_model_fabric_value(cx, fabric, cassette).unwrap()
}

fn value_expr(cx: &mut sim_kernel::Cx, reply: EvalReply) -> Expr {
    reply.value.object().as_expr(cx).unwrap()
}

fn place_runner(cx: &mut sim_kernel::Cx, key: &str, runner: Value) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    cx.call_function(
        &Symbol::qualified("runner", "place"),
        Args::new(vec![
            cx.factory().string(key.to_owned()).unwrap(),
            runner.clone(),
        ]),
    )
    .unwrap();
}

#[test]
fn cache_hit_skips_backend() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = fake_runner(&mut cx, "cached once");
    let cached = model_cached(&mut cx, runner);
    let first_reply = cached
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("same prompt"))
        .unwrap();
    let first = value_expr(&mut cx, first_reply);
    let second_reply = cached
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("same prompt"))
        .unwrap();
    let second = value_expr(&mut cx, second_reply);

    validate_chat_transcript(&first).unwrap();
    validate_chat_transcript(&second).unwrap();
    assert_eq!(map_field(&first, "cache-hit"), Some(&Expr::Bool(false)));
    assert_eq!(map_field(&second, "cache-hit"), Some(&Expr::Bool(true)));
    assert!(flatten_text(&second).contains("cached once"));
}

#[test]
fn backend_error_is_not_recorded() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let missing = model_at(&mut cx, "model-site:missing-cache-error");
    let cached = model_cached(&mut cx, missing);
    let err = cached
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("missing"))
        .err()
        .expect("missing placement key unexpectedly resolved");
    assert!(err.to_string().contains("model-site:missing-cache-error"));

    let err = cached
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("missing"))
        .err()
        .expect("missing placement key unexpectedly replayed as success");
    assert!(err.to_string().contains("model-site:missing-cache-error"));
}

#[test]
fn cassette_replays_with_backend_removed() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let ledger = Arc::new(MemoryLedger::default());
    let runner = fake_runner(&mut cx, "recorded placement");
    place_runner(&mut cx, "model-site:recorded-cache", runner);
    let placed = model_at(&mut cx, "model-site:recorded-cache");
    let cached = cached_with_ledger(&mut cx, placed, ledger.clone());

    let recorded_reply = cached
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("replay prompt"))
        .unwrap();
    let recorded = value_expr(&mut cx, recorded_reply);
    assert_eq!(map_field(&recorded, "cache-hit"), Some(&Expr::Bool(false)));

    let replay_cassette = Arc::new(EvalCassette::from_ledger(ledger).unwrap());
    let missing_backend = model_at(&mut cx, "model-site:backend-removed");
    let replay = cached_model_fabric_value(&mut cx, missing_backend, replay_cassette).unwrap();
    let replayed_reply = replay
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("replay prompt"))
        .unwrap();
    let replayed = value_expr(&mut cx, replayed_reply);

    validate_chat_transcript(&replayed).unwrap();
    assert_eq!(map_field(&replayed, "cache-hit"), Some(&Expr::Bool(true)));
    assert!(flatten_text(&replayed).contains("recorded placement"));
}
