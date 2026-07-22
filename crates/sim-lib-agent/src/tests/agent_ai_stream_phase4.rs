use super::support::{
    as_component, eval_cx, install_agent_lib, install_roundtrip_codecs, request_frame,
    temp_memory_path,
};
use crate::value_from_expr;
use sim_kernel::{Expr, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};
use sim_value::access::field;

#[derive(Default)]
struct CollectSink {
    chunks: Vec<Expr>,
    seen: Vec<FrameKind>,
}

impl StreamSink for CollectSink {
    fn chunk(&mut self, cx: &mut sim_kernel::Cx, frame: ServerFrame) -> sim_kernel::Result<()> {
        self.seen.push(frame.kind.clone());
        let expr = match frame.kind {
            FrameKind::Response => eval_reply_from_frame(cx, &frame)?
                .value
                .object()
                .as_expr(cx)?,
            FrameKind::StreamChunk => frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?,
            FrameKind::StreamStart | FrameKind::StreamEnd => return Ok(()),
            _ => return Ok(()),
        };
        self.chunks.push(expr);
        Ok(())
    }

    fn end(&mut self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<()> {
        Ok(())
    }
}

#[test]
fn scripted_tool_events_stream_before_final() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let tool_call = Expr::Map(vec![
        key_expr("id", Expr::String("call-1".to_owned())),
        key_expr("name", Expr::Symbol(Symbol::new("lookup"))),
        key_expr(
            "arguments",
            Expr::Map(vec![key_expr("query", Expr::String("sim".to_owned()))]),
        ),
    ]);
    let tool_result = Expr::Map(vec![
        key_expr("id", Expr::String("call-1".to_owned())),
        key_expr("name", Expr::Symbol(Symbol::new("lookup"))),
        key_expr("result", Expr::String("found".to_owned())),
    ]);
    let final_response = model_response_expr("tool final");
    let script = Expr::List(vec![
        model_event_expr("start", Vec::new()),
        model_event_expr("tool-call", vec![key_expr("tool-call", tool_call.clone())]),
        model_event_expr(
            "tool-result",
            vec![key_expr("tool-result", tool_result.clone())],
        ),
        model_event_expr("final", vec![key_expr("response", final_response.clone())]),
    ]);

    let script_value = value_from_expr(&mut cx, &Expr::List(vec![script])).unwrap();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("phase4-tools")).unwrap(),
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                script_value,
            ]),
        )
        .unwrap();

    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("tool stream"));
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();

    assert_eq!(sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(sink.seen.last(), Some(&FrameKind::StreamEnd));
    assert!(!sink.seen.contains(&FrameKind::Response));
    let actual_kinds = sink
        .chunks
        .iter()
        .map(|expr| field(expr, "event").unwrap().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        actual_kinds,
        vec![
            Expr::Symbol(Symbol::new("start")),
            Expr::Symbol(Symbol::new("tool-call")),
            Expr::Symbol(Symbol::new("tool-result")),
            Expr::Symbol(Symbol::new("final")),
        ]
    );
    assert_eq!(field(&sink.chunks[1], "tool-call"), Some(&tool_call));
    assert_eq!(field(&sink.chunks[2], "tool-result"), Some(&tool_result));
    assert_eq!(field(&sink.chunks[3], "response"), Some(&final_response));
}

#[test]
fn cache_hit_streams_final_event_with_cache_hit_true() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let key = temp_memory_path("phase4-stream-cache")
        .display()
        .to_string();
    let request = cached_request_expr("cached stream", &key);

    let first = fake_runner_with_text(&mut cx, "phase4-cache", "stored answer");
    let mut first_sink = CollectSink::default();
    let first_request = request_frame(&mut cx, request.clone());
    as_component(&first)
        .stream(&mut cx, first_request, &mut first_sink)
        .unwrap();

    let second = fake_runner_with_text(&mut cx, "phase4-cache", "miss answer");
    let mut second_sink = CollectSink::default();
    let second_request = request_frame(&mut cx, request);
    as_component(&second)
        .stream(&mut cx, second_request, &mut second_sink)
        .unwrap();

    assert_eq!(second_sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(second_sink.seen.last(), Some(&FrameKind::StreamEnd));
    let final_events = second_sink
        .chunks
        .iter()
        .filter(|expr| field(expr, "event") == Some(&Expr::Symbol(Symbol::new("final"))))
        .collect::<Vec<_>>();
    assert_eq!(final_events.len(), 1);
    let response = field(final_events[0], "response").unwrap();
    assert_eq!(field(response, "cache-hit"), Some(&Expr::Bool(true)));
    assert!(format!("{response:?}").contains("stored answer"));
}

fn request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        key_bool("model-request", true),
        key_expr("task", Expr::String(task.to_owned())),
        key_expr("messages", Expr::List(Vec::new())),
    ])
}

fn cached_request_expr(task: &str, key: &str) -> Expr {
    let mut entries = match request_expr(task) {
        Expr::Map(entries) => entries,
        _ => unreachable!(),
    };
    entries.push(key_expr(
        "cache",
        Expr::Map(vec![
            key_expr("mode", Expr::Symbol(Symbol::new("read-through"))),
            key_expr("semantic-key", Expr::String(key.to_owned())),
        ]),
    ));
    Expr::Map(entries)
}

fn fake_runner_with_text(cx: &mut sim_kernel::Cx, name: &str, text: &str) -> sim_kernel::Value {
    let script_value =
        value_from_expr(cx, &Expr::List(vec![Expr::String(text.to_owned())])).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        sim_kernel::Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn model_event_expr(event: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        key_bool("model-event", true),
        key_expr("event", Expr::Symbol(Symbol::new(event))),
        key_expr("runner", Expr::Symbol(Symbol::new("phase4-stream"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr("span-id", Expr::String("span-phase4".to_owned())),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn model_response_expr(text: &str) -> Expr {
    Expr::Map(vec![
        key_bool("model-response", true),
        key_expr("runner", Expr::Symbol(Symbol::new("phase4-stream"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr(
            "content",
            Expr::List(vec![Expr::Map(vec![
                key_expr("type", Expr::Symbol(Symbol::new("text"))),
                key_expr("text", Expr::String(text.to_owned())),
            ])]),
        ),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("stop"))),
    ])
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
