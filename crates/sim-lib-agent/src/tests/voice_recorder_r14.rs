use super::support::{as_component, eval_cx, install_agent_lib, install_test_codec, request_frame};
use crate::Component;
use sim_kernel::{Consistency, Error, Expr, ReadPolicy, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, eval_reply_from_frame};
use sim_lib_stream_core::{PcmPacket, StreamPacket};

#[test]
fn r14_prometheus_scrape_renders_real_exposition() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let recorder = cx
        .call_function(
            &Symbol::qualified("recorder", "prometheus"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":namespace")).unwrap(),
                cx.factory().string("sim_agent".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    notify_trace(
        &mut cx,
        &recorder,
        Some(Symbol::new("worker")),
        Expr::String("draft".to_owned()),
    );
    notify_trace(
        &mut cx,
        &recorder,
        Some(Symbol::new("critic")),
        Expr::String("review".to_owned()),
    );
    notify_trace(
        &mut cx,
        &recorder,
        Some(Symbol::new("tool")),
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("tool")),
            Expr::String("web\"search\nalpha".to_owned()),
        )]),
    );

    let scrape_request = request_frame(
        &mut cx,
        Expr::List(vec![Expr::Symbol(Symbol::new("scrape"))]),
    );
    let scrape_reply = as_component(&recorder)
        .answer(&mut cx, scrape_request)
        .unwrap();
    let scrape = eval_reply_from_frame(&mut cx, &scrape_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let Expr::String(text) = scrape else {
        panic!("expected scrape text, found {scrape:?}");
    };

    assert!(text.contains("# HELP sim_agent_frames_total Frames recorded by the agent fabric."));
    assert!(text.contains("# TYPE sim_agent_frames_total counter"));
    assert!(text.contains("sim_agent_frames_total 3"));
    assert!(text.contains("sim_agent_frames_total{role=\"worker\"} 1"));
    assert!(text.contains("sim_agent_frames_total{role=\"critic\"} 1"));
    assert!(text.contains("# HELP sim_agent_tool_calls_total Tool calls recorded."));
    assert!(text.contains("# TYPE sim_agent_tool_calls_total counter"));
    assert!(text.contains("sim_agent_tool_calls_total{tool=\"web\\\"search\\nalpha\"} 1"));
}

#[test]
fn r14_voice_fallback_is_deterministic_and_labeled() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let tts = cx
        .call_function(
            &Symbol::qualified("voice", "tts"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("narrator".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let tts_request = request_frame(&mut cx, Expr::String("speak now".to_owned()));
    let tts_reply = as_component(&tts).answer(&mut cx, tts_request).unwrap();
    let tts_expr = eval_reply_from_frame(&mut cx, &tts_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(map_bool_field(&tts_expr, "synthetic"));
    assert_eq!(
        map_expr_field(&tts_expr, "audio"),
        Expr::Bytes(b"speak now".to_vec())
    );

    let stt = cx
        .call_function(
            &Symbol::qualified("voice", "stt"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":locale")).unwrap(),
                cx.factory().string("en-GB".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let stt_request = request_frame(&mut cx, Expr::Bytes(b"speak now".to_vec()));
    let stt_reply = as_component(&stt).answer(&mut cx, stt_request).unwrap();
    let stt_expr = eval_reply_from_frame(&mut cx, &stt_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(map_bool_field(&stt_expr, "synthetic"));
    assert_eq!(
        map_expr_field(&stt_expr, "text"),
        Expr::String("speak now".to_owned())
    );
}

#[test]
fn r9_voice_streams_text_data_to_pcm_and_pcm_to_transcript_data() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let tts = cx
        .call_function(
            &Symbol::qualified("voice", "tts"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("streamer".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let model_text = StreamPacket::data(
        Symbol::qualified("stream/data", "model-event"),
        Expr::String("speak as stream".to_owned()),
    );
    let tts_frame = stream_frame(&mut cx, FrameKind::StreamChunk, model_text.to_expr());
    let tts_reply = as_component(&tts).answer(&mut cx, tts_frame).unwrap();
    assert_eq!(tts_reply.kind, FrameKind::StreamChunk);
    let StreamPacket::Pcm(pcm) = StreamPacket::try_from(
        tts_reply
            .decode_expr(&mut cx, ReadPolicy::default())
            .unwrap(),
    )
    .unwrap() else {
        panic!("expected PCM packet");
    };
    assert_eq!(pcm.channels(), 1);
    assert!(pcm.frames() > 0);

    let stt = cx
        .call_function(
            &Symbol::qualified("voice", "stt"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":locale")).unwrap(),
                cx.factory().string("en-US".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let pcm_packet = StreamPacket::Pcm(PcmPacket::i16(1, 2, vec![65, 66]).unwrap());
    let stt_frame = stream_frame(&mut cx, FrameKind::StreamChunk, pcm_packet.to_expr());
    let stt_reply = as_component(&stt).answer(&mut cx, stt_frame).unwrap();
    assert_eq!(stt_reply.kind, FrameKind::StreamChunk);
    let StreamPacket::Data(transcript) = StreamPacket::try_from(
        stt_reply
            .decode_expr(&mut cx, ReadPolicy::default())
            .unwrap(),
    )
    .unwrap() else {
        panic!("expected transcript data packet");
    };
    assert_eq!(
        transcript.kind,
        Symbol::qualified("stream/data", "voice-transcript")
    );
    assert!(matches!(
        map_expr_field(&transcript.payload, "text"),
        Expr::String(text) if !text.is_empty()
    ));
}

#[test]
fn r14_voice_command_path_requires_capability_and_runs_command() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let tts = cx
        .call_function(
            &Symbol::qualified("voice", "tts"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("shell".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory().string("cat".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let reflected = as_component(&tts).reflect(&mut cx).unwrap();
    assert_eq!(
        map_expr_field(&reflected, "command"),
        Expr::String("cat".to_owned())
    );

    let denied_request = request_frame(&mut cx, Expr::String("hello".to_owned()));
    let denied = as_component(&tts).answer(&mut cx, denied_request);
    assert!(matches!(
        denied,
        Err(Error::CapabilityDenied { capability })
            if capability == sim_kernel::CapabilityName::new("voice")
    ));

    cx.grant_named("voice");
    let tts_request = request_frame(&mut cx, Expr::String("hello".to_owned()));
    let tts_reply = as_component(&tts).answer(&mut cx, tts_request).unwrap();
    let tts_expr = eval_reply_from_frame(&mut cx, &tts_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(!map_bool_field(&tts_expr, "synthetic"));
    assert_eq!(
        map_expr_field(&tts_expr, "audio"),
        Expr::Bytes(b"hello".to_vec())
    );

    let stt = cx
        .call_function(
            &Symbol::qualified("voice", "stt"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory().string("cat".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let stt_request = request_frame(&mut cx, Expr::Bytes(b"spoken".to_vec()));
    let stt_reply = as_component(&stt).answer(&mut cx, stt_request).unwrap();
    let stt_expr = eval_reply_from_frame(&mut cx, &stt_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(!map_bool_field(&stt_expr, "synthetic"));
    assert_eq!(
        map_expr_field(&stt_expr, "text"),
        Expr::String("spoken".to_owned())
    );
}

fn notify_trace(
    cx: &mut sim_kernel::Cx,
    recorder: &sim_kernel::Value,
    role: Option<Symbol>,
    payload: Expr,
) {
    let mut notify = ServerFrame::from_expr(
        cx,
        Symbol::qualified("codec", "binary"),
        FrameKind::Notify,
        &payload,
        sim_kernel::Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap();
    notify.envelope.role = role;
    as_component(recorder).answer(cx, notify).unwrap();
}

fn stream_frame(cx: &mut sim_kernel::Cx, kind: FrameKind, payload: Expr) -> ServerFrame {
    ServerFrame::from_expr(
        cx,
        Symbol::qualified("codec", "binary"),
        kind,
        &payload,
        Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap()
}

fn map_expr_field(expr: &Expr, key: &str) -> Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map expr, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing key {key} in {expr:?}"))
}

fn map_bool_field(expr: &Expr, key: &str) -> bool {
    match map_expr_field(expr, key) {
        Expr::Bool(value) => value,
        other => panic!("expected bool field {key}, found {other:?}"),
    }
}
