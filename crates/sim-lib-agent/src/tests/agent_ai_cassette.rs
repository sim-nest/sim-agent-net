use super::support::{
    as_component, eval_cx, install_agent_lib, install_test_codec, request_frame, temp_memory_path,
};
use crate::{Component, memory::append_memory_log};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Expr, ReadPolicy, Symbol};
use sim_lib_server::EvalSite;

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

fn trace_entry(task_id: &str, kind: sim_lib_server::FrameKind, payload: Expr) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("trace-entry")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task-id")),
            Expr::String(task_id.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("kind")),
            Expr::Symbol(kind.as_symbol()),
        ),
        (Expr::Symbol(Symbol::new("payload")), payload),
    ])
}

fn map_field<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn without_cache_hit(expr: &Expr) -> Expr {
    let Expr::Map(entries) = expr else {
        return expr.clone();
    };
    Expr::Map(
        entries
            .iter()
            .filter(|(key, _)| *key != Expr::Symbol(Symbol::new("cache-hit")))
            .cloned()
            .collect(),
    )
}

#[test]
fn a5_phase4_runner_cassette_replays_fake_runner_journal() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let request_expr = model_request_expr("replay me");
    let request_payload = request_frame(&mut cx, request_expr.clone())
        .decode_expr(&mut cx, ReadPolicy::default())
        .unwrap();

    let fake = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::String("recorded answer".to_owned())]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let fake_request = request_frame(&mut cx, request_expr.clone());
    let reply_payload = as_component(&fake)
        .answer(&mut cx, fake_request)
        .unwrap()
        .decode_expr(&mut cx, ReadPolicy::default())
        .unwrap();
    let recorded_response = map_field(&reply_payload, "value").unwrap().clone();

    let journal = temp_memory_path("cassette-runner");
    append_memory_log(
        &journal,
        &trace_entry(
            "task-1",
            sim_lib_server::FrameKind::Request,
            request_payload,
        ),
    )
    .unwrap();
    append_memory_log(
        &journal,
        &trace_entry("task-1", sim_lib_server::FrameKind::Response, reply_payload),
    )
    .unwrap();

    let cassette = cx
        .call_function(
            &Symbol::qualified("runner", "cassette"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":journal")).unwrap(),
                cx.factory().string(journal.display().to_string()).unwrap(),
            ]),
        )
        .unwrap();

    assert!(as_component(&cassette).capabilities().is_empty());

    let replayed = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                request_expr,
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("runner", "cassette"))),
                    args: vec![
                        Expr::Symbol(Symbol::new(":journal")),
                        Expr::String(journal.display().to_string()),
                    ],
                },
            ],
        })
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    validate_chat_transcript(&replayed).unwrap();
    assert_eq!(map_field(&replayed, "cache-hit"), Some(&Expr::Bool(true)));
    assert_eq!(without_cache_hit(&replayed), recorded_response);

    let _ = std::fs::remove_file(journal);
}

#[test]
fn a5_phase4_runner_cassette_strict_miss_returns_structured_error() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let journal = temp_memory_path("cassette-miss");
    let response = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                model_request_expr("missing"),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("runner", "cassette"))),
                    args: vec![
                        Expr::Symbol(Symbol::new(":journal")),
                        Expr::String(journal.display().to_string()),
                    ],
                },
            ],
        })
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    validate_chat_transcript(&response).unwrap();
    assert_eq!(map_field(&response, "cache-hit"), Some(&Expr::Bool(false)));
    let message = format!("{response:?}");
    assert!(message.contains("cassette miss"));

    let _ = std::fs::remove_file(journal);
}
