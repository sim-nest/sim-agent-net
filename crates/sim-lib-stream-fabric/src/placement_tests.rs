use std::sync::Arc;

use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Expr, Symbol};
use sim_lib_server::{
    FrameEnvelope, FrameKind, LocalEvalSite, ServerAddress, stream_frame_to_expr,
};
use sim_lib_stream_core::{
    BufferPolicy, ClockDomain, LatencyClass, PcmPacket, PlacedFragment, RateContract,
    StreamDirection, StreamEdge, StreamEnvelope, StreamItem, StreamMedia, StreamMetadata,
    StreamPacket, TransportProfile, stream_remote_network_capability,
    stream_remote_preview_capability, stream_remote_render_capability,
};

use crate::{
    PlacementResourceLimits, ServerPlacementRequest, placement_capability_names,
    placement_host_device_capability, placement_run_node_on_server_capability,
    server_buffered_preview_profile, server_placement_frame_operation_symbols,
    server_placement_refusal_diagnostic_kind, server_placement_refusal_symbol,
    server_placement_request_symbol, server_placement_result_symbol,
    server_placement_stream_frames, server_render_return_profile, server_site_symbol,
};

#[test]
fn server_placement_runs_node_and_returns_declared_render_audio() {
    let mut cx = cx();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let fragment = fragment_with_audio_edge();

    let frames = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment,
        ServerPlacementRequest::render_return(placement_report()),
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap();

    assert_eq!(frames.first().unwrap().kind, FrameKind::StreamStart);
    assert_eq!(frames.last().unwrap().kind, FrameKind::StreamEnd);
    assert_eq!(
        server_placement_frame_operation_symbols(),
        [
            server_placement_request_symbol(),
            server_placement_result_symbol(),
            server_placement_refusal_symbol(),
        ]
    );

    let request = chunk_envelope(&mut cx, &frames[1]);
    let StreamPacket::Data(request_packet) = request.packet() else {
        panic!("expected request packet");
    };
    assert_eq!(request_packet.kind, server_placement_request_symbol());
    assert_eq!(
        map_field_symbol(&request_packet.payload, "site"),
        Some(server_site_symbol())
    );
    assert_eq!(
        map_field_symbol(&request_packet.payload, "declared-latency"),
        Some(LatencyClass::OfflineRender.symbol())
    );
    assert_eq!(
        map_list_strings(&request_packet.payload, "required-capabilities"),
        Some(vec![
            "stream.placement.run-node-on-server".to_owned(),
            "stream.remote.network".to_owned(),
            "stream.remote.render".to_owned(),
        ])
    );
    assert_eq!(
        map_nested_string(
            &request_packet.payload,
            "resource-limits",
            "max-cpu-time-ms"
        ),
        Some("5000".to_owned())
    );
    assert!(map_list_len(&request_packet.payload, "output-edges").unwrap() > 0);

    let result = chunk_envelope(&mut cx, &frames[2]);
    let StreamPacket::Data(result_packet) = result.packet() else {
        panic!("expected result packet");
    };
    assert_eq!(result_packet.kind, server_placement_result_symbol());
    assert_eq!(
        map_field(&result_packet.payload, "node-result"),
        Some(&Expr::String("analysis-result".to_owned()))
    );

    let pcm = frames
        .iter()
        .filter(|frame| frame.kind == FrameKind::StreamChunk)
        .map(|frame| chunk_envelope(&mut cx, frame))
        .find(|envelope| matches!(envelope.packet(), StreamPacket::Pcm(_)))
        .expect("server placement returned a PCM chunk");
    assert_eq!(pcm.media(), StreamMedia::Pcm);
    assert_eq!(pcm.profile().name(), server_render_return_profile().name());
    assert_eq!(pcm.profile().latency_class(), LatencyClass::OfflineRender);
}

#[test]
fn server_placement_refuses_realtime_pinned_nodes_over_frames() {
    let mut cx = cx();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let request =
        ServerPlacementRequest::buffered_preview(placement_report()).with_realtime_pin(true);

    let frames = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment_with_audio_edge(),
        request,
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap();

    assert_eq!(frames.first().unwrap().kind, FrameKind::StreamStart);
    assert_eq!(frames.last().unwrap().kind, FrameKind::StreamEnd);
    let refusal = chunk_envelope(&mut cx, &frames[2]);
    assert_eq!(
        refusal.diagnostics(),
        &[server_placement_refusal_diagnostic_kind()]
    );
    let StreamPacket::Diagnostic(packet) = refusal.packet() else {
        panic!("expected refusal diagnostic packet");
    };
    assert_eq!(packet.kind(), &server_placement_refusal_diagnostic_kind());
    assert!(packet.message().contains("realtime-pinned"));
    assert!(packet.message().contains("stream/site/server"));
}

#[test]
fn server_placement_requires_run_node_capability() {
    let mut cx = cx_without_placement_run_grant();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let err = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment_with_audio_edge(),
        ServerPlacementRequest::render_return(placement_report()),
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == placement_run_node_on_server_capability()
    ));
}

#[test]
fn server_placement_resource_cutoff_emits_refusal() {
    let mut cx = cx();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let request = ServerPlacementRequest::render_return(placement_report()).with_resource_limits(
        PlacementResourceLimits {
            max_stream_frames: 1,
            ..PlacementResourceLimits::default()
        },
    );

    let frames = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment_with_audio_edge(),
        request,
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap();

    assert_eq!(frames.first().unwrap().kind, FrameKind::StreamStart);
    assert_eq!(frames.last().unwrap().kind, FrameKind::StreamEnd);
    let refusal = chunk_envelope(&mut cx, &frames[2]);
    let StreamPacket::Diagnostic(packet) = refusal.packet() else {
        panic!("expected resource cutoff diagnostic packet");
    };
    assert_eq!(packet.kind(), &server_placement_refusal_diagnostic_kind());
    assert!(packet.message().contains("resource cutoff"));
    assert!(packet.message().contains("stream-size"));
}

#[test]
fn placement_reports_are_redacted_before_frame_recording() {
    let mut cx = cx();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let frames = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment_with_audio_edge(),
        ServerPlacementRequest::render_return(private_placement_report()),
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap();

    let request = chunk_envelope(&mut cx, &frames[1]);
    let StreamPacket::Data(request_packet) = request.packet() else {
        panic!("expected request packet");
    };
    let report = map_field(&request_packet.payload, "placement-report")
        .expect("request carries placement report");

    assert!(expr_contains_text(report, "[redacted placement data]"));
    assert!(expr_contains_text(report, "[redacted placement payload]"));
    assert!(!expr_contains_text(report, "private-path=session.sim"));
    assert!(!expr_contains_text(report, "host=studio.local"));
    assert!(!expr_contains_text(report, "token=abc123"));
}

#[test]
fn host_device_placement_requires_host_device_capability() {
    let mut cx = cx();
    let site = LocalEvalSite::new(ServerAddress::Local, vec![lisp_codec()]);
    let fragment = PlacedFragment::new(
        Symbol::qualified("test", "host-device"),
        Expr::String("host-device CoreAudio Built-in Output".to_owned()),
    );
    let err = server_placement_stream_frames(
        &mut cx,
        &site,
        fragment,
        ServerPlacementRequest::buffered_preview(placement_report()),
        lisp_codec(),
        FrameEnvelope::default(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == placement_host_device_capability()
    ));
}

#[test]
fn placement_capability_names_are_stable() {
    let names = placement_capability_names()
        .into_iter()
        .map(|capability| capability.as_str().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "stream.placement.run-node-on-server",
            "stream.placement.run-node-on-lan-peer",
            "stream.remote.render",
            "stream.host.device",
        ]
    );
}

#[test]
fn server_profiles_declare_preview_and_render_latency() {
    assert_eq!(
        server_buffered_preview_profile().latency_class(),
        LatencyClass::BufferedPreview
    );
    assert_eq!(
        server_render_return_profile().latency_class(),
        LatencyClass::OfflineRender
    );
}

fn cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let codec_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&sim_codec_lisp::LispCodecLib::new(codec_id).unwrap())
        .unwrap();
    cx.grant(stream_remote_network_capability());
    cx.grant(stream_remote_preview_capability());
    cx.grant(stream_remote_render_capability());
    cx.grant(placement_run_node_on_server_capability());
    cx
}

fn cx_without_placement_run_grant() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let codec_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&sim_codec_lisp::LispCodecLib::new(codec_id).unwrap())
        .unwrap();
    cx.grant(stream_remote_network_capability());
    cx.grant(stream_remote_preview_capability());
    cx.grant(stream_remote_render_capability());
    cx
}

fn fragment_with_audio_edge() -> PlacedFragment {
    PlacedFragment::new(
        Symbol::qualified("test", "server-analysis"),
        Expr::String("analysis-result".to_owned()),
    )
    .with_output_edge(audio_edge())
}

fn audio_edge() -> StreamEdge {
    let metadata = StreamMetadata::new(
        Symbol::qualified("stream/output", "analysis-audio"),
        StreamMedia::Pcm,
        StreamDirection::Source,
        ClockDomain::ServerFrame.symbol(),
        BufferPolicy::bounded(2).unwrap(),
    );
    let item = StreamItem::new(StreamPacket::Pcm(
        PcmPacket::i16(1, 2, vec![100, -100]).unwrap(),
    ));
    let envelope = StreamEnvelope::from_item_with_profile(
        &metadata,
        0,
        &item,
        TransportProfile::memory_local(),
    )
    .unwrap();
    StreamEdge::new(
        Symbol::new("audio-out"),
        RateContract::new(ClockDomain::ServerFrame, LatencyClass::OfflineRender, None),
        metadata,
    )
    .with_envelopes(vec![envelope])
}

fn placement_report() -> Expr {
    Expr::Map(vec![
        field(
            "planner",
            Expr::Symbol(Symbol::qualified("test", "planner")),
        ),
        field("assignment", Expr::Symbol(server_site_symbol())),
    ])
}

fn private_placement_report() -> Expr {
    let limit = sim_lib_stream_core::StreamRemoteLimits::default().max_binary_payload_bytes;
    Expr::Map(vec![
        field("path", Expr::String("private-path=session.sim".to_owned())),
        field("host", Expr::String("host=studio.local".to_owned())),
        field("secret", Expr::String("token=abc123".to_owned())),
        field("payload", Expr::Bytes(vec![0; limit + 1])),
    ])
}

fn chunk_envelope(cx: &mut Cx, frame: &sim_lib_server::ServerFrame) -> StreamEnvelope {
    let expr = stream_frame_to_expr(cx, frame).unwrap().unwrap();
    StreamEnvelope::try_from(expr).unwrap()
}

use sim_value::access::field as map_field;

use sim_value::access::field_sym as map_field_symbol;

fn map_list_len(expr: &Expr, name: &str) -> Option<usize> {
    match map_field(expr, name)? {
        Expr::List(items) => Some(items.len()),
        _ => None,
    }
}

fn map_list_strings(expr: &Expr, name: &str) -> Option<Vec<String>> {
    match map_field(expr, name)? {
        Expr::List(items) => items
            .iter()
            .map(|item| match item {
                Expr::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn map_nested_string(expr: &Expr, field_name: &str, nested_name: &str) -> Option<String> {
    let nested = map_field(expr, field_name)?;
    match map_field(nested, nested_name)? {
        Expr::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn expr_contains_text(expr: &Expr, expected: &str) -> bool {
    match expr {
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.as_qualified_str().contains(expected),
        Expr::String(value) => value.contains(expected),
        Expr::Bytes(_) => false,
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().any(|item| expr_contains_text(item, expected))
        }
        Expr::Map(entries) => entries.iter().any(|(key, value)| {
            expr_contains_text(key, expected) || expr_contains_text(value, expected)
        }),
        Expr::Call { operator, args } => {
            expr_contains_text(operator, expected)
                || args.iter().any(|arg| expr_contains_text(arg, expected))
        }
        Expr::Infix { left, right, .. } => {
            expr_contains_text(left, expected) || expr_contains_text(right, expected)
        }
        Expr::Prefix { arg, .. } | Expr::Postfix { arg, .. } => expr_contains_text(arg, expected),
        Expr::Quote { expr, .. } => expr_contains_text(expr, expected),
        Expr::Annotated { expr, annotations } => {
            expr_contains_text(expr, expected)
                || annotations.iter().any(|(key, value)| {
                    key.as_qualified_str().contains(expected) || expr_contains_text(value, expected)
                })
        }
        Expr::Extension { tag, payload } => {
            tag.as_qualified_str().contains(expected) || expr_contains_text(payload, expected)
        }
        _ => false,
    }
}

use sim_value::build::entry as field;

fn lisp_codec() -> Symbol {
    Symbol::qualified("codec", "lisp")
}
