use super::support::{
    as_component, eval_cx, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use crate::value_from_expr;
use sim_kernel::{Expr, Symbol, seq_close_value, seq_next_value};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};

#[derive(Default)]
struct CollectSink {
    chunks: Vec<Expr>,
    seen: Vec<FrameKind>,
    ended: bool,
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
            FrameKind::StreamStart => return Ok(()),
            FrameKind::StreamEnd => {
                self.ended = true;
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.chunks.push(expr);
        Ok(())
    }

    fn end(&mut self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<()> {
        self.ended = true;
        Ok(())
    }
}

#[test]
fn a5_phase7_fake_runner_streams_scripted_events() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let final_response = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("model-response")),
            Expr::Bool(true),
        ),
        (
            Expr::Symbol(Symbol::new("runner")),
            Expr::Symbol(Symbol::new("fake-stream")),
        ),
        (
            Expr::Symbol(Symbol::new("model")),
            Expr::String("runner/fake".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("content")),
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("type")),
                    Expr::Symbol(Symbol::new("text")),
                ),
                (
                    Expr::Symbol(Symbol::new("text")),
                    Expr::String("streamed final".to_owned()),
                ),
            ])]),
        ),
        (
            Expr::Symbol(Symbol::new("stop-reason")),
            Expr::Symbol(Symbol::new("stop")),
        ),
    ]);
    let script = Expr::List(vec![
        Expr::Map(vec![
            (Expr::Symbol(Symbol::new("model-event")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("event")),
                Expr::Symbol(Symbol::new("start")),
            ),
            (
                Expr::Symbol(Symbol::new("runner")),
                Expr::Symbol(Symbol::new("fake-stream")),
            ),
            (
                Expr::Symbol(Symbol::new("model")),
                Expr::String("runner/fake".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("span-id")),
                Expr::String("s-1".to_owned()),
            ),
        ]),
        Expr::Map(vec![
            (Expr::Symbol(Symbol::new("model-event")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("event")),
                Expr::Symbol(Symbol::new("delta")),
            ),
            (
                Expr::Symbol(Symbol::new("runner")),
                Expr::Symbol(Symbol::new("fake-stream")),
            ),
            (
                Expr::Symbol(Symbol::new("model")),
                Expr::String("runner/fake".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("span-id")),
                Expr::String("s-1".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("text")),
                Expr::String("chunk".to_owned()),
            ),
        ]),
        Expr::Map(vec![
            (Expr::Symbol(Symbol::new("model-event")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("event")),
                Expr::Symbol(Symbol::new("final")),
            ),
            (
                Expr::Symbol(Symbol::new("runner")),
                Expr::Symbol(Symbol::new("fake-stream")),
            ),
            (
                Expr::Symbol(Symbol::new("model")),
                Expr::String("runner/fake".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("span-id")),
                Expr::String("s-1".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("response")),
                final_response.clone(),
            ),
        ]),
    ]);

    let script_value = value_from_expr(&mut cx, &Expr::List(vec![script])).unwrap();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("fake-stream")).unwrap(),
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                script_value,
            ]),
        )
        .unwrap();

    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("stream"));
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();
    assert_eq!(sink.chunks.len(), 3);
    assert_eq!(
        sink.seen,
        vec![
            FrameKind::StreamStart,
            FrameKind::StreamChunk,
            FrameKind::StreamChunk,
            FrameKind::StreamChunk,
            FrameKind::StreamEnd,
        ]
    );
    assert!(sink.ended);
    assert!(format!("{:?}", sink.chunks[1]).contains("delta"));
    assert!(format!("{:?}", sink.chunks[2]).contains("streamed final"));
}

#[test]
fn phase0_fake_runner_stream_chunks_cover_current_model_event_kinds() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let usage = Expr::Map(vec![
        key_expr("input-tokens", Expr::String("12".to_owned())),
        key_expr("output-tokens", Expr::String("4".to_owned())),
    ]);
    let tool_call = Expr::Map(vec![
        key_expr("id", Expr::String("call-1".to_owned())),
        key_expr("name", Expr::Symbol(Symbol::new("lookup"))),
        key_expr(
            "arguments",
            Expr::Map(vec![key_expr("query", Expr::String("sim".to_owned()))]),
        ),
    ]);
    let final_response = phase0_model_response_expr("streamed final");
    let script = Expr::List(vec![
        phase0_model_event_expr("start", Vec::new()),
        phase0_model_event_expr(
            "delta",
            vec![key_expr("text", Expr::String("chunk".to_owned()))],
        ),
        phase0_model_event_expr("usage", vec![key_expr("usage", usage.clone())]),
        phase0_model_event_expr("tool-call", vec![key_expr("tool-call", tool_call.clone())]),
        phase0_model_event_expr("final", vec![key_expr("response", final_response.clone())]),
    ]);

    let script_value = value_from_expr(&mut cx, &Expr::List(vec![script])).unwrap();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("phase0-stream")).unwrap(),
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                script_value,
            ]),
        )
        .unwrap();

    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("phase 0 stream"));
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
            Expr::Symbol(Symbol::new("delta")),
            Expr::Symbol(Symbol::new("usage")),
            Expr::Symbol(Symbol::new("tool-call")),
            Expr::Symbol(Symbol::new("final")),
        ]
    );
    assert_eq!(field(&sink.chunks[2], "usage"), Some(&usage));
    assert_eq!(field(&sink.chunks[3], "tool-call"), Some(&tool_call));
    assert_eq!(field(&sink.chunks[4], "response"), Some(&final_response));
}

#[cfg(feature = "runner-process")]
#[test]
fn a5_phase7_process_line_text_streams_deltas_and_final() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("ai-runner");
    cx.grant_named("ai-runner-local");
    cx.grant_named("host.process");

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "process"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":protocol")).unwrap(),
                cx.factory().string("line-text".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory()
                    .string("printf 'one\\ntwo\\n'".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();

    let mut sink = CollectSink::default();
    let request = request_frame(&mut cx, request_expr("lines"));
    as_component(&runner)
        .stream(&mut cx, request, &mut sink)
        .unwrap();
    assert_eq!(sink.seen.first(), Some(&FrameKind::StreamStart));
    assert_eq!(sink.seen.last(), Some(&FrameKind::StreamEnd));
    assert!(
        sink.chunks
            .iter()
            .any(|expr| format!("{expr:?}").contains("start"))
    );
    assert!(
        sink.chunks
            .iter()
            .any(|expr| format!("{expr:?}").contains("delta"))
    );
    assert!(
        sink.chunks
            .iter()
            .any(|expr| format!("{expr:?}").contains("one"))
    );
    assert!(
        sink.chunks
            .iter()
            .any(|expr| format!("{expr:?}").contains("two"))
    );
    assert!(
        sink.chunks
            .iter()
            .any(|expr| format!("{expr:?}").contains("final"))
    );
}

#[test]
fn a5_phase7_agent_stream_returns_event_chunks() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner_script = value_from_expr(
        &mut cx,
        &Expr::List(vec![Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("model-response")),
                Expr::Bool(true),
            ),
            (
                Expr::Symbol(Symbol::new("runner")),
                Expr::Symbol(Symbol::new("agent-fake")),
            ),
            (
                Expr::Symbol(Symbol::new("model")),
                Expr::String("runner/fake".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("content")),
                Expr::List(vec![Expr::Map(vec![
                    (
                        Expr::Symbol(Symbol::new("type")),
                        Expr::Symbol(Symbol::new("text")),
                    ),
                    (
                        Expr::Symbol(Symbol::new("text")),
                        Expr::String("agent stream".to_owned()),
                    ),
                ])]),
            ),
            (
                Expr::Symbol(Symbol::new("stop-reason")),
                Expr::Symbol(Symbol::new("stop")),
            ),
        ])]),
    )
    .unwrap();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("agent-fake")).unwrap(),
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                runner_script,
            ]),
        )
        .unwrap();
    let request_value = value_from_expr(&mut cx, &request_expr("agent stream")).unwrap();
    let stream = cx
        .call_function(
            &Symbol::qualified("agent", "stream"),
            sim_kernel::Args::new(vec![runner, request_value]),
        )
        .unwrap();
    let chunk = seq_next_value(&mut cx, &stream).unwrap().unwrap();
    let chunk = chunk.value().object().as_expr(&mut cx).unwrap();
    assert!(format!("{chunk:?}").contains("model-event"));
    assert!(format!("{chunk:?}").contains("agent stream"));
    seq_close_value(&mut cx, &stream).unwrap();
    assert!(seq_next_value(&mut cx, &stream).unwrap().is_none());
}

#[test]
fn a5_phase7_recorder_stores_model_event_frames() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let recorder = cx
        .call_function(
            &Symbol::qualified("recorder", "journal"),
            sim_kernel::Args::new(Vec::new()),
        )
        .unwrap();
    let event = Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-event")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("event")),
            Expr::Symbol(Symbol::new("delta")),
        ),
        (
            Expr::Symbol(Symbol::new("runner")),
            Expr::Symbol(Symbol::new("recorded-runner")),
        ),
        (
            Expr::Symbol(Symbol::new("model")),
            Expr::String("runner/fake".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("span-id")),
            Expr::String("span-1".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("text")),
            Expr::String("record me".to_owned()),
        ),
    ]);
    let mut notify = ServerFrame::from_expr(
        &mut cx,
        Symbol::qualified("codec", "binary"),
        FrameKind::Notify,
        &event,
        sim_kernel::Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap();
    notify.envelope.role = Some(Symbol::new("runner"));
    as_component(&recorder).answer(&mut cx, notify).unwrap();

    let snapshot = request_frame(
        &mut cx,
        Expr::List(vec![Expr::Symbol(Symbol::new("snapshot"))]),
    );
    let reply = as_component(&recorder).answer(&mut cx, snapshot).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(format!("{expr:?}").contains("model-event"));
    assert!(format!("{expr:?}").contains("record me"));
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

fn phase0_model_event_expr(event: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        key_bool("model-event", true),
        key_expr("event", Expr::Symbol(Symbol::new(event))),
        key_expr("runner", Expr::Symbol(Symbol::new("phase0-stream"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr("span-id", Expr::String("span-phase0".to_owned())),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn phase0_model_response_expr(text: &str) -> Expr {
    Expr::Map(vec![
        key_bool("model-response", true),
        key_expr("runner", Expr::Symbol(Symbol::new("phase0-stream"))),
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

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
            Some(value)
        }
        _ => None,
    })
}
