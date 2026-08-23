use std::sync::Arc;

use sim_kernel::{
    Cx, Datum, DatumStore, DefaultFactory, Error, Event, EventKind, Expr, NoopEvalPolicy,
    ObserveMode, Ref, Symbol, Term, read_eval_capability,
};
use sim_lib_rank::order_score::rank_frontier_data_kind;
use sim_lib_rank::{Nat, RankFrontier, RankFrontierPayload, ScoredOrdinal};
use sim_lib_server::{
    FrameEnvelope, FrameKind, ServerFrame, stream_chunk_frame_from_expr, stream_frame_from_expr,
    stream_frame_to_expr,
};
use sim_lib_stream_core::{
    BufferPolicy, ClockDomain, MidiPacket, MidiPacketEvent, PcmPacket, StreamDirection,
    StreamEnvelope, StreamItem, StreamMedia, StreamMetadata, StreamPacket, StreamValue,
    TransportProfile, stream_remote_network_capability,
};

use crate::{
    StreamFrameLimits, cassette_to_stream_frames, event_buffer_to_stream, expr_chunk_event,
    realize_stream_events, refused_profile_diagnostic_kind, remote_error_diagnostic_kind,
    stream_frames_to_cassette, stream_frames_to_stream, stream_limit_diagnostic_kind,
    stream_realize_request, stream_to_frames, stream_to_frames_with_limits,
    stream_to_frames_with_profile,
};

#[test]
fn local_realize_of_midi_pipeline_yields_packet_events() {
    let mut cx = cx();
    let stream = midi_stream("stream/local-midi");
    let request = stream_realize_request(term("midi-pipeline"), Some(8));

    let events = realize_stream_events(&mut cx, &request, &stream).unwrap();

    assert_eq!(request.observe, ObserveMode::Events);
    assert_eq!(chunk_count(&events), 2);
    assert!(matches!(
        events.first().unwrap().kind,
        EventKind::Started { .. }
    ));
    assert!(matches!(events.last().unwrap().kind, EventKind::Done));
    let decoded =
        event_buffer_to_stream(&mut cx, midi_metadata("stream/local-midi-copy"), events).unwrap();
    assert_midi_packets(decoded.take_packets(4).unwrap(), 2);
}

#[test]
fn local_realize_of_pcm_pipeline_yields_packet_events() {
    let mut cx = cx();
    let stream = pcm_stream("stream/local-pcm");
    let request = stream_realize_request(term("pcm-pipeline"), Some(8));

    let events = realize_stream_events(&mut cx, &request, &stream).unwrap();

    assert_eq!(chunk_count(&events), 2);
    let decoded =
        event_buffer_to_stream(&mut cx, pcm_metadata("stream/local-pcm-copy"), events).unwrap();
    assert_pcm_packets(decoded.take_packets(4).unwrap(), 2);
}

#[test]
fn metadata_matches_on_both_sides() {
    let mut cx = cx();
    let metadata = midi_metadata("stream/remote-midi");
    let stream = midi_stream_with_metadata(metadata.clone());

    let frames = stream_to_frames(&mut cx, &stream, lisp_codec()).unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();

    assert_eq!(remote.metadata(), &metadata);
    assert_midi_packets(remote.take_packets(8).unwrap(), 2);
}

#[test]
fn stream_frames_carry_remote_stream_envelopes() {
    let mut cx = cx();
    let metadata = midi_metadata("stream/enveloped-midi");
    let stream = midi_stream_with_metadata(metadata.clone());

    let frames = stream_to_frames(&mut cx, &stream, lisp_codec()).unwrap();
    let expr = stream_frame_to_expr(&mut cx, &frames[1]).unwrap().unwrap();
    let envelope = StreamEnvelope::try_from(expr).unwrap();

    assert_eq!(envelope.stream_id(), metadata.id());
    assert_eq!(envelope.sequence(), 0);
    assert_eq!(
        envelope.profile().name(),
        &Symbol::qualified("stream/profile", "remote-stream-fabric")
    );
    assert!(matches!(envelope.packet(), StreamPacket::Midi(_)));
}

#[test]
fn server_frame_streams_record_to_cassette_and_replay() {
    let mut cx = cx();
    let stream = midi_stream("stream/cassette-remote-midi");
    let frames = stream_to_frames_with_profile(
        &mut cx,
        &stream,
        lisp_codec(),
        FrameEnvelope::default(),
        TransportProfile::lan_midi_control(),
    )
    .unwrap();

    let cassette = stream_frames_to_cassette(&mut cx, &frames).unwrap();
    let replay_frames =
        cassette_to_stream_frames(&mut cx, &cassette, lisp_codec(), FrameEnvelope::default())
            .unwrap();
    let replay = stream_frames_to_stream(&mut cx, &replay_frames).unwrap();

    assert_eq!(cassette.envelopes().len(), 2);
    assert_midi_packets(replay.take_packets(8).unwrap(), 2);
}

#[test]
fn remote_stream_frames_require_remote_network_capability() {
    let mut cx = cx_without_remote_network();
    let stream = midi_stream("stream/no-remote-grant");
    let err = match stream_to_frames(&mut cx, &stream, lisp_codec()) {
        Ok(_) => panic!("remote stream frames unexpectedly encoded without capability"),
        Err(err) => err,
    };
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == stream_remote_network_capability()
    ));
}

#[test]
fn compatibility_packet_chunk_frames_still_decode() {
    let mut cx = cx();
    let start = stream_frame_from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::StreamStart,
        &midi_metadata("stream/compat-packet").table_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();
    let chunk = stream_chunk_frame_from_expr(
        &mut cx,
        lisp_codec(),
        &StreamPacket::Midi(midi_packet(0)).to_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();

    let remote = stream_frames_to_stream(&mut cx, &[start, chunk]).unwrap();

    assert_midi_packets(remote.take_packets(8).unwrap(), 1);
}

#[test]
fn remote_stream_chunk_payload_rejects_read_eval() {
    let mut cx = cx();
    let start = stream_frame_from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::StreamStart,
        &data_metadata("stream/read-eval-denied").table_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();
    let chunk = ServerFrame::new(
        lisp_codec(),
        FrameKind::StreamChunk,
        FrameEnvelope::default(),
        b"#. 1".to_vec(),
    );

    let err = match stream_frames_to_stream(&mut cx, &[start, chunk]) {
        Ok(_) => panic!("read-eval stream chunk unexpectedly decoded"),
        Err(err) => err,
    };

    assert_read_eval_denied(err);
}

#[test]
fn remote_realtime_audio_profile_becomes_refused_profile_diagnostic() {
    let mut cx = cx();
    let stream = pcm_stream("stream/realtime-refused");

    let frames = stream_to_frames_with_profile(
        &mut cx,
        &stream,
        lisp_codec(),
        FrameEnvelope::default(),
        TransportProfile::realtime_local_audio(),
    )
    .unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();
    let item = remote.next_packet().unwrap().unwrap();

    let StreamPacket::Diagnostic(packet) = item.packet() else {
        panic!("expected refused profile diagnostic packet");
    };
    assert_eq!(packet.kind(), &refused_profile_diagnostic_kind());
    assert!(packet.message().contains("realtime-local-audio"));
    assert!(packet.message().contains("lan-buffered-audio-preview"));
}

#[test]
fn buffered_preview_profiles_are_accepted_over_remote_frames() {
    assert_profile_round_trips_pcm_packets(
        "stream/buffered-preview",
        TransportProfile::buffered_pcm_preview(),
    );
    assert_profile_round_trips_pcm_packets(
        "stream/lan-buffered-preview",
        TransportProfile::lan_buffered_audio_preview(),
    );
}

#[test]
fn lan_midi_control_and_render_return_profiles_are_accepted_over_remote_frames() {
    let mut cx = cx();
    let stream = midi_stream("stream/lan-midi-control");
    let frames = stream_to_frames_with_profile(
        &mut cx,
        &stream,
        lisp_codec(),
        FrameEnvelope::default(),
        TransportProfile::lan_midi_control(),
    )
    .unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();
    assert_midi_packets(remote.take_packets(8).unwrap(), 2);

    assert_profile_round_trips_pcm_packets(
        "stream/lan-render-return",
        TransportProfile::lan_render_return(),
    );
}

#[test]
fn stream_frame_limits_emit_diagnostic_chunks() {
    assert_limit_diagnostic(
        StreamFrameLimits {
            max_frame_payload_bytes: 1,
            ..StreamFrameLimits::default()
        },
        "frame-size",
    );
    assert_limit_diagnostic(
        StreamFrameLimits {
            max_stream_frames: 0,
            ..StreamFrameLimits::default()
        },
        "stream-size",
    );
    assert_limit_diagnostic(
        StreamFrameLimits {
            max_inflight_frames: 1,
            ..StreamFrameLimits::default()
        },
        "inflight-frame",
    );
    assert_limit_diagnostic(
        StreamFrameLimits {
            max_duration_ms: 1,
            max_rate_hz: 1,
            ..StreamFrameLimits::default()
        },
        "duration-rate",
    );
}

#[test]
fn cancellation_makes_later_next_deterministic_nil() {
    let mut cx = cx();
    let stream = pcm_stream("stream/cancel-source");
    let frames = stream_to_frames(&mut cx, &stream, lisp_codec()).unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();

    remote.cancel().unwrap();

    assert!(remote.next_packet().unwrap().is_none());
    assert!(remote.next_packet().unwrap().is_none());
    assert!(remote.is_done().unwrap());
}

#[test]
fn remote_error_becomes_remote_error_diagnostic_packet() {
    let mut cx = cx();
    let start = stream_frame_from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::StreamStart,
        &midi_metadata("stream/error").table_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();
    let error = ServerFrame::from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::Error,
        &Expr::String("remote failed".to_owned()),
        sim_kernel::Consistency::default(),
        Vec::new(),
        false,
    )
    .unwrap();

    let remote = stream_frames_to_stream(&mut cx, &[start, error]).unwrap();
    let item = remote.next_packet().unwrap().unwrap();

    let StreamPacket::Diagnostic(packet) = item.packet() else {
        panic!("expected diagnostic packet");
    };
    assert_eq!(packet.kind(), &remote_error_diagnostic_kind());
    assert!(packet.message().contains("remote failed"));
}

#[test]
fn no_new_stream_frame_kind_exists() {
    let model = include_str!("../../sim-lib-server/src/frame/model.rs");
    let stream_variants = model
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_suffix(',')
                .filter(|variant| variant.starts_with("Stream"))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        stream_variants,
        vec!["StreamStart", "StreamChunk", "StreamEnd"]
    );
}

#[test]
fn raw_model_event_expr_chunks_become_data_packets() {
    let mut cx = cx();
    let raw = raw_model_event_expr();
    let event = chunk_event_from_expr(&mut cx, raw.clone());
    let decoded = event_buffer_to_stream(
        &mut cx,
        data_metadata("stream/raw-model-event"),
        vec![event],
    )
    .unwrap();
    assert_data_packet(
        decoded.next_packet().unwrap().unwrap(),
        Symbol::qualified("stream/data", "expr"),
        &raw,
    );

    let start = stream_frame_from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::StreamStart,
        &data_metadata("stream/raw-model-frame").table_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();
    let chunk = stream_chunk_frame_from_expr(&mut cx, lisp_codec(), &raw, FrameEnvelope::default())
        .unwrap();
    assert_eq!(stream_frame_to_expr(&mut cx, &chunk).unwrap(), Some(raw));
    let remote = stream_frames_to_stream(&mut cx, &[start, chunk]).unwrap();
    assert_data_packet(
        remote.next_packet().unwrap().unwrap(),
        Symbol::qualified("stream/data", "expr"),
        &raw_model_event_expr(),
    );
}

#[test]
fn expr_chunk_event_preserves_custom_data_kind() {
    let mut cx = cx();
    let raw = raw_model_event_expr();
    let event = expr_chunk_event(
        &mut cx,
        Ref::Symbol(Symbol::qualified("stream/test", "expr-chunk")),
        7,
        Symbol::qualified("stream/data", "model-event"),
        raw.clone(),
    )
    .unwrap();
    let decoded =
        event_buffer_to_stream(&mut cx, data_metadata("stream/expr-chunk"), vec![event]).unwrap();

    assert_data_packet(
        decoded.next_packet().unwrap().unwrap(),
        Symbol::qualified("stream/data", "model-event"),
        &raw,
    );
}

#[test]
fn data_stream_frames_round_trip_to_equivalent_packets() {
    let mut cx = cx();
    let packet = StreamPacket::model_event(raw_model_event_expr());
    let stream = StreamValue::pull(
        data_metadata("stream/data-roundtrip"),
        vec![StreamItem::new(packet.clone())],
    );

    let frames = stream_to_frames(&mut cx, &stream, lisp_codec()).unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();

    assert_eq!(
        remote.take_packets(4).unwrap(),
        vec![StreamItem::new(packet)]
    );
}

#[test]
fn rank_data_stream_frames_record_and_replay() {
    let mut cx = cx();
    let frontier = RankFrontier::new(
        Symbol::qualified("rank-frontier", "fabric"),
        vec![
            ScoredOrdinal {
                ordinal: Nat::from(2_u64),
                score: 9,
            },
            ScoredOrdinal {
                ordinal: Nat::from(5_u64),
                score: 7,
            },
        ],
        8,
    );
    let expected = frontier
        .data_stream(RankFrontierPayload::OrdinalContent)
        .take_packets(8)
        .unwrap();
    let stream = frontier.data_stream(RankFrontierPayload::OrdinalContent);

    let frames = stream_to_frames(&mut cx, &stream, lisp_codec()).unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();

    assert_eq!(remote.metadata().media(), StreamMedia::Data);
    assert_eq!(remote.take_packets(8).unwrap(), expected);
    assert_eq!(data_kind(&expected[0]), Some(rank_frontier_data_kind()));
}

#[test]
fn non_datum_expr_chunk_frame_becomes_diagnostic_packet() {
    let mut cx = cx();
    let start = stream_frame_from_expr(
        &mut cx,
        lisp_codec(),
        FrameKind::StreamStart,
        &data_metadata("stream/non-datum").table_expr(),
        FrameEnvelope::default(),
    )
    .unwrap();
    let chunk = stream_chunk_frame_from_expr(
        &mut cx,
        lisp_codec(),
        &Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("stream/test", "opaque"))),
            args: vec![Expr::String("payload".to_owned())],
        },
        FrameEnvelope::default(),
    )
    .unwrap();

    let remote = stream_frames_to_stream(&mut cx, &[start, chunk]).unwrap();
    let item = remote.next_packet().unwrap().unwrap();

    let StreamPacket::Diagnostic(packet) = item.packet() else {
        panic!("expected diagnostic packet");
    };
    assert_eq!(
        packet.kind(),
        &Symbol::qualified("stream/fabric", "ChunkPayloadError")
    );
    assert!(packet.message().contains("datum conversion failed"));
}

fn cx() -> Cx {
    let mut cx = cx_without_remote_network();
    cx.grant(stream_remote_network_capability());
    cx
}

fn cx_without_remote_network() -> Cx {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let codec_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&sim_codec_lisp::LispCodecLib::new(codec_id).unwrap())
        .unwrap();
    cx
}

fn lisp_codec() -> Symbol {
    Symbol::qualified("codec", "lisp")
}

fn term(name: &str) -> Term {
    Term::Ref(Ref::Symbol(Symbol::qualified("stream/test", name)))
}

fn midi_stream(id: &str) -> StreamValue {
    midi_stream_with_metadata(midi_metadata(id))
}

fn midi_stream_with_metadata(metadata: StreamMetadata) -> StreamValue {
    StreamValue::pull(
        metadata,
        vec![
            StreamItem::new(StreamPacket::Midi(midi_packet(0))),
            StreamItem::new(StreamPacket::Midi(midi_packet(240))),
        ],
    )
}

fn pcm_stream(id: &str) -> StreamValue {
    StreamValue::pull(
        pcm_metadata(id),
        vec![
            StreamItem::new(StreamPacket::Pcm(
                PcmPacket::i16(2, 1, vec![1, -1]).unwrap(),
            )),
            StreamItem::new(StreamPacket::Pcm(
                PcmPacket::i16(2, 1, vec![2, -2]).unwrap(),
            )),
        ],
    )
}

fn midi_packet(ticks: i64) -> MidiPacket {
    MidiPacket::new(vec![
        MidiPacketEvent::new(ticks, 480, vec![0x90, 60, 100]).unwrap(),
    ])
    .unwrap()
}

fn midi_metadata(id: &str) -> StreamMetadata {
    StreamMetadata::new(
        Symbol::new(id),
        StreamMedia::Midi,
        StreamDirection::Source,
        Symbol::qualified("clock", "midi"),
        BufferPolicy::bounded(8).unwrap(),
    )
}

fn pcm_metadata(id: &str) -> StreamMetadata {
    StreamMetadata::new(
        Symbol::new(id),
        StreamMedia::Pcm,
        StreamDirection::Source,
        ClockDomain::Sample.symbol(),
        BufferPolicy::bounded(8).unwrap(),
    )
}

fn data_metadata(id: &str) -> StreamMetadata {
    StreamMetadata::new(
        Symbol::new(id),
        StreamMedia::Data,
        StreamDirection::Source,
        ClockDomain::ServerFrame.symbol(),
        BufferPolicy::bounded(8).unwrap(),
    )
}

fn chunk_count(events: &[sim_kernel::Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::Chunk { .. }))
        .count()
}

fn assert_midi_packets(items: Vec<StreamItem>, expected: usize) {
    assert_eq!(items.len(), expected);
    for item in items {
        assert!(matches!(item.packet(), StreamPacket::Midi(_)));
    }
}

fn assert_pcm_packets(items: Vec<StreamItem>, expected: usize) {
    assert_eq!(items.len(), expected);
    for item in items {
        assert!(matches!(item.packet(), StreamPacket::Pcm(_)));
    }
}

fn assert_profile_round_trips_pcm_packets(id: &str, profile: TransportProfile) {
    let mut cx = cx();
    let stream = pcm_stream(id);
    let frames = stream_to_frames_with_profile(
        &mut cx,
        &stream,
        lisp_codec(),
        FrameEnvelope::default(),
        profile,
    )
    .unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();

    assert_pcm_packets(remote.take_packets(8).unwrap(), 2);
}

fn assert_limit_diagnostic(limits: StreamFrameLimits, expected_message: &str) {
    let mut cx = cx();
    let stream = midi_stream("stream/limit");
    let frames = stream_to_frames_with_limits(
        &mut cx,
        &stream,
        lisp_codec(),
        FrameEnvelope::default(),
        TransportProfile::remote_stream_fabric(),
        limits,
    )
    .unwrap();
    let remote = stream_frames_to_stream(&mut cx, &frames).unwrap();
    let items = remote.take_packets(8).unwrap();
    let packet = items
        .iter()
        .find_map(|item| match item.packet() {
            StreamPacket::Diagnostic(packet) => Some(packet),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected limit diagnostic packet"));
    assert_eq!(packet.kind(), &stream_limit_diagnostic_kind());
    assert!(packet.message().contains(expected_message));
}

fn assert_read_eval_denied(error: Error) {
    assert!(
        matches!(error, Error::CapabilityDenied { ref capability } if capability == &read_eval_capability()),
        "expected read-eval denial, got {error:?}",
    );
}

fn chunk_event_from_expr(cx: &mut Cx, expr: Expr) -> Event {
    let datum = Datum::try_from(expr).unwrap();
    let payload = Ref::Content(cx.datum_store_mut().intern(datum).unwrap());
    Event::new(
        Ref::Symbol(Symbol::qualified("stream/test", "raw-model-event")),
        0,
        Vec::new(),
        EventKind::Chunk { payload },
    )
    .unwrap()
}

fn raw_model_event_expr() -> Expr {
    Expr::Map(vec![
        key_bool("model-event", true),
        key_expr("event", Expr::Symbol(Symbol::new("delta"))),
        key_expr("runner", Expr::Symbol(Symbol::new("raw-runner"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr("span-id", Expr::String("span-raw".to_owned())),
        key_expr("text", Expr::String("not a stream packet".to_owned())),
    ])
}

fn assert_data_packet(item: StreamItem, expected_kind: Symbol, expected_payload: &Expr) {
    let StreamPacket::Data(packet) = item.packet() else {
        panic!("expected data packet");
    };
    assert_eq!(packet.kind, expected_kind);
    assert_eq!(&packet.payload, expected_payload);
}

fn data_kind(item: &StreamItem) -> Option<Symbol> {
    match item.packet() {
        StreamPacket::Data(packet) => Some(packet.kind.clone()),
        _ => None,
    }
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
