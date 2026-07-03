use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_test_codec, request_frame,
    temp_memory_path,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Expr, ReadPolicy, Symbol};
use sim_lib_server::EvalSite;
use std::time::Duration;

fn model_request_expr(task: &str, cache: Expr) -> Expr {
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
        (Expr::Symbol(Symbol::new("cache")), cache),
    ])
}

fn reordered_model_request_expr(task: &str, cache: Expr) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("cache")), cache),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
    ])
}

fn cache_policy(key: &str) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("mode")),
            Expr::Symbol(Symbol::new("read-through")),
        ),
        (
            Expr::Symbol(Symbol::new("semantic-key")),
            Expr::String(key.to_owned()),
        ),
    ])
}

fn cache_policy_with_path(key: &str, path: &str) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("mode")),
            Expr::Symbol(Symbol::new("read-through")),
        ),
        (
            Expr::Symbol(Symbol::new("semantic-key")),
            Expr::String(key.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("path")),
            Expr::String(path.to_owned()),
        ),
    ])
}

fn cache_policy_with_ttl(key: &str, ttl: &str) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("mode")),
            Expr::Symbol(Symbol::new("read-through")),
        ),
        (
            Expr::Symbol(Symbol::new("semantic-key")),
            Expr::String(key.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("ttl")),
            Expr::String(ttl.to_owned()),
        ),
    ])
}

fn fake_runner(cx: &mut sim_kernel::Cx, model: &str, answer: &str) -> sim_kernel::Value {
    let script = Expr::List(vec![Expr::String(answer.to_owned())]);
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            cx.factory().expr(script).unwrap(),
        ]),
    )
    .unwrap()
}

fn answer_with_runner(cx: &mut sim_kernel::Cx, runner: &sim_kernel::Value, request: Expr) -> Expr {
    let frame = request_frame(cx, request);
    as_component(runner)
        .answer(cx, frame)
        .unwrap()
        .decode_expr(cx, ReadPolicy::default())
        .unwrap()
        .as_map_field("value")
        .cloned()
        .unwrap()
}

trait MapField {
    fn as_map_field(&self, key: &str) -> Option<&Expr>;
}

impl MapField for Expr {
    fn as_map_field(&self, key: &str) -> Option<&Expr> {
        let Expr::Map(entries) = self else {
            return None;
        };
        entries.iter().find_map(|(field, value)| match field {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
            _ => None,
        })
    }
}

#[test]
fn a6_phase4_cache_key_is_stable_across_map_order_and_includes_model() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let key = temp_memory_path("cache-key").display().to_string();

    let first = fake_runner(&mut cx, "model-a", "first");
    let first_response = answer_with_runner(
        &mut cx,
        &first,
        model_request_expr("same task", cache_policy(&key)),
    );
    validate_chat_transcript(&first_response).unwrap();
    assert_eq!(
        first_response.as_map_field("cache-hit"),
        Some(&Expr::Bool(false))
    );

    let second = fake_runner(&mut cx, "model-a", "second");
    let second_response = answer_with_runner(
        &mut cx,
        &second,
        reordered_model_request_expr("same task", cache_policy(&key)),
    );
    validate_chat_transcript(&second_response).unwrap();
    assert_eq!(
        second_response.as_map_field("cache-hit"),
        Some(&Expr::Bool(true))
    );
    assert!(flatten_text(&second_response).contains("first"));

    let other_model = fake_runner(&mut cx, "model-b", "model scoped");
    let other_response = answer_with_runner(
        &mut cx,
        &other_model,
        model_request_expr("same task", cache_policy(&key)),
    );
    assert_eq!(
        other_response.as_map_field("cache-hit"),
        Some(&Expr::Bool(false))
    );
    assert!(flatten_text(&other_response).contains("model scoped"));
}

#[test]
fn a6_phase4_cache_key_includes_tools() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let tools_key = temp_memory_path("cache-tools").display().to_string();
    let with_tools = Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("tool descriptor task".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (
            Expr::Symbol(Symbol::new("tools")),
            Expr::List(vec![Expr::String("sum".to_owned())]),
        ),
        (Expr::Symbol(Symbol::new("cache")), cache_policy(&tools_key)),
    ]);
    let without_tools = model_request_expr("tool descriptor task", cache_policy(&tools_key));
    let runner_three = fake_runner(&mut cx, "tool-model", "with tools");
    answer_with_runner(&mut cx, &runner_three, with_tools);
    let runner_four = fake_runner(&mut cx, "tool-model", "without tools");
    let response = answer_with_runner(&mut cx, &runner_four, without_tools);
    assert_eq!(response.as_map_field("cache-hit"), Some(&Expr::Bool(false)));
    assert!(flatten_text(&response).contains("without tools"));
}

#[test]
fn a6_phase4_cache_ttl_ignores_stale_entries() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let key = temp_memory_path("cache-ttl").display().to_string();
    let request = model_request_expr("ttl task", cache_policy_with_ttl(&key, "1ms"));

    let first = fake_runner(&mut cx, "ttl-model", "stale answer");
    answer_with_runner(&mut cx, &first, request.clone());
    std::thread::sleep(Duration::from_millis(5));

    let second = fake_runner(&mut cx, "ttl-model", "fresh answer");
    let response = answer_with_runner(&mut cx, &second, request);
    assert_eq!(response.as_map_field("cache-hit"), Some(&Expr::Bool(false)));
    assert!(flatten_text(&response).contains("fresh answer"));
}

#[test]
fn a6_phase4_non_idempotent_tool_continuation_is_not_cached() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let key = temp_memory_path("cache-tool-continuation")
        .display()
        .to_string();
    let request = Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("continue after tool".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (
            Expr::Symbol(Symbol::new("tool-results")),
            Expr::List(vec![Expr::String("side effect".to_owned())]),
        ),
        (Expr::Symbol(Symbol::new("cache")), cache_policy(&key)),
    ]);

    let first = fake_runner(&mut cx, "tool-continuation", "first continuation");
    answer_with_runner(&mut cx, &first, request.clone());
    let second = fake_runner(&mut cx, "tool-continuation", "second continuation");
    let response = answer_with_runner(&mut cx, &second, request);
    assert!(response.as_map_field("cache-hit").is_none());
    assert!(flatten_text(&response).contains("second continuation"));
}

#[test]
fn a6_phase4_persistent_cache_writes_require_capability() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let path = temp_memory_path("persistent-model-cache");
    let request = model_request_expr(
        "persistent task",
        cache_policy_with_path("persistent-key", &path.display().to_string()),
    );
    let runner = fake_runner(&mut cx, "persistent-model", "would write");
    let frame = request_frame(&mut cx, request.clone());
    let denied = as_component(&runner).answer(&mut cx, frame).unwrap_err();
    assert!(
        matches!(denied, sim_kernel::Error::CapabilityDenied { capability } if capability == sim_kernel::CapabilityName::new("ai-runner-cache"))
    );

    cx.grant_named("ai-runner-cache");
    let writer = fake_runner(&mut cx, "persistent-model", "persisted");
    let response = answer_with_runner(&mut cx, &writer, request.clone());
    assert_eq!(response.as_map_field("cache-hit"), Some(&Expr::Bool(false)));

    let reader = fake_runner(&mut cx, "persistent-model", "miss");
    let response = answer_with_runner(&mut cx, &reader, request);
    validate_chat_transcript(&response).unwrap();
    assert_eq!(response.as_map_field("cache-hit"), Some(&Expr::Bool(true)));
    assert!(flatten_text(&response).contains("persisted"));

    let _ = std::fs::remove_file(path);
}
