use std::sync::Arc;

use sim_codec_lisp::LispCodecLib;
use sim_kernel::{Consistency, DefaultFactory, EagerPolicy, EvalReply};

use super::*;
use crate::{FrameEnvelope, server_frame_from_reply};

fn cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx
}

fn codec() -> Symbol {
    Symbol::qualified("codec", "lisp")
}

fn response_frame(cx: &mut Cx, text: &str) -> ServerFrame {
    let value = cx.factory().string(text.to_owned()).unwrap();
    server_frame_from_reply(
        cx,
        &codec(),
        EvalReply {
            value,
            diagnostics: Vec::new(),
            trace: None,
        },
        Consistency::LocalFirst,
    )
    .unwrap()
}

fn chunk_frame(cx: &mut Cx, text: &str) -> ServerFrame {
    stream_chunk_frame_from_expr(
        cx,
        codec(),
        &Expr::String(text.to_owned()),
        FrameEnvelope {
            consistency: Consistency::LocalFirst,
            ..FrameEnvelope::default()
        },
    )
    .unwrap()
}

#[test]
fn stream_frame_helpers_decode_payloads_and_skip_boundaries() {
    let mut cx = cx();
    let start = ServerFrame::new(
        codec(),
        FrameKind::StreamStart,
        FrameEnvelope::default(),
        Vec::new(),
    );
    assert_eq!(stream_frame_to_expr(&mut cx, &start).unwrap(), None);
    assert!(stream_frame_to_value(&mut cx, &start).unwrap().is_none());

    let chunk = chunk_frame(&mut cx, "chunk");
    assert_eq!(
        stream_frame_to_expr(&mut cx, &chunk).unwrap(),
        Some(Expr::String("chunk".to_owned()))
    );
    assert_eq!(
        stream_frame_to_value(&mut cx, &chunk)
            .unwrap()
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("chunk".to_owned())
    );

    let response = response_frame(&mut cx, "fallback");
    assert_eq!(
        stream_frame_to_expr(&mut cx, &response).unwrap(),
        Some(Expr::String("fallback".to_owned()))
    );
    assert_eq!(
        stream_frame_to_value(&mut cx, &response)
            .unwrap()
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("fallback".to_owned())
    );

    let end = ServerFrame::new(
        codec(),
        FrameKind::StreamEnd,
        FrameEnvelope::default(),
        Vec::new(),
    );
    assert_eq!(stream_frame_to_expr(&mut cx, &end).unwrap(), None);
    assert!(stream_frame_to_value(&mut cx, &end).unwrap().is_none());
}

#[test]
fn stream_frame_builders_roundtrip_payloads_and_reject_non_stream_kinds() {
    let mut cx = cx();
    let payload = Expr::String("payload".to_owned());
    let envelope = FrameEnvelope {
        consistency: Consistency::LocalFirst,
        trace: true,
        ..FrameEnvelope::default()
    };
    let start = stream_frame_from_expr(
        &mut cx,
        codec(),
        FrameKind::StreamStart,
        &payload,
        envelope.clone(),
    )
    .unwrap();
    assert_eq!(start.kind, FrameKind::StreamStart);
    assert_eq!(start.envelope, envelope);
    assert_eq!(stream_frame_to_expr(&mut cx, &start).unwrap(), None);

    let chunk =
        stream_chunk_frame_from_expr(&mut cx, codec(), &payload, FrameEnvelope::default()).unwrap();
    assert_eq!(chunk.kind, FrameKind::StreamChunk);
    assert_eq!(
        stream_frame_to_expr(&mut cx, &chunk).unwrap(),
        Some(payload.clone())
    );

    let end = stream_end_frame(codec(), FrameEnvelope::default());
    assert_eq!(end.kind, FrameKind::StreamEnd);
    assert!(stream_frame_to_expr(&mut cx, &end).unwrap().is_none());

    let err = stream_frame_from_expr(
        &mut cx,
        codec(),
        FrameKind::Response,
        &payload,
        FrameEnvelope::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("cannot build frame kind response"));
}

#[test]
fn buffered_sink_handles_response_fallback_and_idempotent_end() {
    let mut cx = cx();
    let handle = Arc::new(StreamHandle::default());
    let mut sink = BufferedStreamSink::new(handle.clone());

    let fallback = response_frame(&mut cx, "fallback");
    sink.chunk(&mut cx, fallback).unwrap();
    assert_eq!(handle.buffered_len(), 1);
    sink.end(&mut cx).unwrap();
    assert!(handle.is_done());
    sink.end(&mut cx).unwrap();
    assert!(handle.is_done());
    assert_eq!(
        handle
            .next(&mut cx)
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("fallback".to_owned())
    );
}

#[test]
fn buffered_sink_closes_on_stream_end_before_sink_end() {
    let mut cx = cx();
    let handle = Arc::new(StreamHandle::default());
    let mut sink = BufferedStreamSink::new(handle.clone());

    sink.chunk(
        &mut cx,
        ServerFrame::new(
            codec(),
            FrameKind::StreamStart,
            FrameEnvelope::default(),
            Vec::new(),
        ),
    )
    .unwrap();
    let chunk = chunk_frame(&mut cx, "chunk");
    sink.chunk(&mut cx, chunk).unwrap();
    sink.chunk(
        &mut cx,
        ServerFrame::new(
            codec(),
            FrameKind::StreamEnd,
            FrameEnvelope::default(),
            Vec::new(),
        ),
    )
    .unwrap();

    assert!(handle.is_done());
    assert_eq!(handle.buffered_len(), 1);
    sink.end(&mut cx).unwrap();
    assert!(handle.is_done());
    assert_eq!(handle.buffered_len(), 1);
    assert_eq!(
        handle
            .next(&mut cx)
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("chunk".to_owned())
    );
}
