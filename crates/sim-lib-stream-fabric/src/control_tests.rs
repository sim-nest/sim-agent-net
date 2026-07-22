use std::sync::Arc;

use sim_kernel::{Cx, DefaultFactory, Expr, NoopEvalPolicy, Symbol};
use sim_lib_server::FrameEnvelope;
use sim_lib_stream_core::{
    BufferPolicy, ClockDomain, StreamDirection, StreamEnvelope, StreamFaultKind, StreamFaultSpec,
    StreamItem, StreamMedia, StreamMetadata, StreamPacket, TransportProfile,
    stream_cancel_capability, stream_open_capability, stream_push_capability,
    stream_read_capability, stream_remote_network_capability, stream_stats_capability,
};

use crate::{
    StreamControl, stream_control_cancel_symbol, stream_control_close_symbol,
    stream_control_fault_symbol, stream_control_frame_from_control, stream_control_from_frame,
    stream_control_metadata_symbol, stream_control_next_symbol, stream_control_open_symbol,
    stream_control_operation_symbols, stream_control_push_symbol,
    stream_control_required_capability, stream_control_stats_symbol,
};

#[test]
fn stream_control_operation_frames_round_trip() {
    let mut cx = cx();
    let metadata = data_metadata("stream/control");
    let envelope = StreamEnvelope::from_item_with_profile(
        &metadata,
        0,
        &StreamItem::new(StreamPacket::data(
            Symbol::qualified("stream/data", "expr"),
            Expr::String("payload".to_owned()),
        )),
        TransportProfile::remote_stream_fabric(),
    )
    .unwrap();
    let controls = vec![
        StreamControl::Open {
            stream_id: metadata.id().clone(),
            metadata: metadata.clone(),
        },
        StreamControl::Next {
            stream_id: metadata.id().clone(),
            limit: Some(4),
        },
        StreamControl::Push {
            stream_id: metadata.id().clone(),
            envelope: Box::new(envelope),
        },
        StreamControl::Close {
            stream_id: metadata.id().clone(),
        },
        StreamControl::Cancel {
            stream_id: metadata.id().clone(),
        },
        StreamControl::Stats {
            stream_id: metadata.id().clone(),
        },
        StreamControl::Metadata {
            stream_id: metadata.id().clone(),
        },
        StreamControl::Fault {
            stream_id: metadata.id().clone(),
            fault: StreamFaultSpec::new(StreamFaultKind::Timeout, 2),
        },
    ];

    assert_eq!(
        stream_control_operation_symbols(),
        [
            stream_control_open_symbol(),
            stream_control_next_symbol(),
            stream_control_push_symbol(),
            stream_control_close_symbol(),
            stream_control_cancel_symbol(),
            stream_control_stats_symbol(),
            stream_control_metadata_symbol(),
            stream_control_fault_symbol(),
        ]
    );
    assert_eq!(
        controls
            .iter()
            .map(stream_control_required_capability)
            .collect::<Vec<_>>(),
        [
            stream_open_capability(),
            stream_read_capability(),
            stream_push_capability(),
            stream_cancel_capability(),
            stream_cancel_capability(),
            stream_stats_capability(),
            stream_stats_capability(),
            stream_stats_capability(),
        ]
    );
    for control in controls {
        let frame = stream_control_frame_from_control(
            &mut cx,
            lisp_codec(),
            &control,
            FrameEnvelope::default(),
        )
        .unwrap();
        let decoded = stream_control_from_frame(&mut cx, &frame).unwrap();
        assert_eq!(decoded, control);
    }
}

#[test]
fn stream_control_frames_require_operation_capability() {
    let mut cx = cx_without_stream_operation_grants();
    cx.grant(stream_remote_network_capability());
    let metadata = data_metadata("stream/control-capability");
    let control = StreamControl::Open {
        stream_id: metadata.id().clone(),
        metadata,
    };

    let err = match stream_control_frame_from_control(
        &mut cx,
        lisp_codec(),
        &control,
        FrameEnvelope::default(),
    ) {
        Ok(_) => panic!("stream control frame unexpectedly encoded without operation grant"),
        Err(err) => err,
    };

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == stream_open_capability()
    ));
}

fn cx() -> Cx {
    let mut cx = cx_without_stream_operation_grants();
    cx.grant(stream_remote_network_capability());
    cx.grant(stream_open_capability());
    cx.grant(stream_read_capability());
    cx.grant(stream_push_capability());
    cx.grant(stream_cancel_capability());
    cx.grant(stream_stats_capability());
    cx
}

fn cx_without_stream_operation_grants() -> Cx {
    let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    let codec_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&sim_codec_lisp::LispCodecLib::new(codec_id).unwrap())
        .unwrap();
    cx
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

fn lisp_codec() -> Symbol {
    Symbol::qualified("codec", "lisp")
}
