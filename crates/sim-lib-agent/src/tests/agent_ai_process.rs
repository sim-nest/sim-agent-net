use super::support::{as_component, eval_cx, flatten_text, install_agent_lib, request_frame};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, eval_reply_from_frame};

#[test]
fn a5_phase5_runner_process_json_stdio_replies_and_requires_capabilities() {
    let mut cx = eval_cx();
    super::support::install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "process"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("local-json")).unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory().string("local/stdin".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory()
                    .string(
                        "cat >/dev/null; printf '%s' '{\"$expr\":\"map\",\"entries\":[{\"key\":{\"$expr\":\"symbol\",\"name\":\"model-response\"},\"value\":{\"$expr\":\"bool\",\"value\":true}},{\"key\":{\"$expr\":\"symbol\",\"name\":\"runner\"},\"value\":{\"$expr\":\"symbol\",\"name\":\"local-json\"}},{\"key\":{\"$expr\":\"symbol\",\"name\":\"model\"},\"value\":{\"$expr\":\"string\",\"value\":\"local/stdin\"}},{\"key\":{\"$expr\":\"symbol\",\"name\":\"content\"},\"value\":{\"$expr\":\"list\",\"items\":[{\"$expr\":\"map\",\"entries\":[{\"key\":{\"$expr\":\"symbol\",\"name\":\"type\"},\"value\":{\"$expr\":\"symbol\",\"name\":\"text\"}},{\"key\":{\"$expr\":\"symbol\",\"name\":\"text\"},\"value\":{\"$expr\":\"string\",\"value\":\"fixture ok\"}}]}]}},{\"key\":{\"$expr\":\"symbol\",\"name\":\"stop-reason\"},\"value\":{\"$expr\":\"symbol\",\"name\":\"stop\"}}]}'".to_owned(),
                    )
                    .unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(
        &mut cx,
        Expr::Map(vec![
            (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("task")),
                Expr::String("hello".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("messages")),
                Expr::List(Vec::new()),
            ),
        ]),
    );
    let denied = as_component(&runner)
        .answer(&mut cx, request.clone())
        .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("ai-runner")
    ));

    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");
    cx.grant_named("host.process");
    let reply = as_component(&runner).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("fixture ok"));
}

#[test]
fn a5_phase5_runner_process_returns_error_transcripts_for_timeouts_and_oversize_output() {
    let mut cx = eval_cx();
    super::support::install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");
    cx.grant_named("host.process");

    let timed = cx
        .call_function(
            &Symbol::qualified("runner", "process"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory()
                    .string("sleep 1; printf late".to_owned())
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":timeout")).unwrap(),
                cx.factory().string("50ms".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let timeout_request = request_frame(&mut cx, request_expr("wait"));
    let timeout_reply = as_component(&timed)
        .answer(&mut cx, timeout_request)
        .unwrap();
    let timeout_expr = eval_reply_from_frame(&mut cx, &timeout_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&timeout_expr).unwrap();
    assert!(flatten_text(&timeout_expr).contains("timed out after 50ms"));
    assert!(flatten_text(&timeout_expr).contains("error"));

    let oversized = cx
        .call_function(
            &Symbol::qualified("runner", "process"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory().string("printf 'abcdef'".to_owned()).unwrap(),
                cx.factory()
                    .symbol(Symbol::new(":max-output-bytes"))
                    .unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "3".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let oversize_request = request_frame(&mut cx, request_expr("trim"));
    let oversize_reply = as_component(&oversized)
        .answer(&mut cx, oversize_request)
        .unwrap();
    let oversize_expr = eval_reply_from_frame(&mut cx, &oversize_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&oversize_expr).unwrap();
    assert!(flatten_text(&oversize_expr).contains("exceeded max output bytes 3"));
    assert!(flatten_text(&oversize_expr).contains("error"));
}

fn request_expr(task: &str) -> Expr {
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
