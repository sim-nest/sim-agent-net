use std::{io::Cursor, path::PathBuf, sync::Arc};

use sim_kernel::{Consistency, EvalRequest, Expr, ReadPolicy, Symbol, eval_remote_capability};

use crate::{
    EvalSite, FrameEnvelope, FrameKind, LocalEvalSite, ServerAddress, ServerFrame,
    eval_request_from_frame, server_frame_from_request,
};

use super::super::{
    ConnectionTransport, LocalTransport, TransportEndpoint, connect_transport_site,
    connect_transport_site_with_loopback, decode_transport_frame, encode_transport_frame,
    read_frame_from, register_endpoint, require_connect_capabilities, require_start_capabilities,
    route_frame_bytes, unregister_endpoint, write_frame_to,
};
use super::support::{codecs, cx};

#[test]
fn transport_frame_round_trips_negotiate_header() {
    let frame = ServerFrame {
        version: 1,
        codec: Symbol::qualified("codec", "binary"),
        msg_id: Some(7),
        correlate: Some(3),
        kind: FrameKind::Negotiate {
            codecs: vec![
                Symbol::qualified("codec", "binary"),
                Symbol::qualified("codec", "lisp"),
            ],
        },
        envelope: FrameEnvelope::default(),
        payload: b"abc".to_vec(),
    };

    let decoded = decode_transport_frame(&encode_transport_frame(&frame).unwrap()).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn stream_helpers_round_trip_transport_frames() {
    let frame = ServerFrame {
        version: 1,
        codec: Symbol::qualified("codec", "binary"),
        msg_id: Some(7),
        correlate: Some(3),
        kind: FrameKind::Negotiate {
            codecs: vec![Symbol::qualified("codec", "binary")],
        },
        envelope: FrameEnvelope::default(),
        payload: b"abc".to_vec(),
    };

    let mut cursor = Cursor::new(Vec::new());
    write_frame_to(&mut cursor, &frame).unwrap();
    cursor.set_position(0);

    let decoded = read_frame_from(&mut cursor).unwrap().unwrap();
    assert_eq!(decoded, frame);
    assert!(read_frame_from(&mut cursor).unwrap().is_none());
}

#[test]
fn transport_frame_rejects_truncated_prefix() {
    let err = decode_transport_frame(&[0, 1, 2]).unwrap_err();
    assert!(format!("{err}").contains("truncated transport frame prefix"));
}

#[test]
fn stream_helpers_reject_truncated_prefix() {
    let mut cursor = Cursor::new(vec![0, 1, 2]);
    let err = read_frame_from(&mut cursor).unwrap_err();
    assert!(format!("{err}").contains("truncated transport frame prefix"));
}

#[test]
fn transport_frame_rejects_oversized_lengths_before_allocation() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(u32::MAX).to_be_bytes());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    let err = decode_transport_frame(&bytes).unwrap_err();
    assert!(format!("{err}").contains("size limit"));
}

#[test]
fn local_transport_routes_through_eval_site() {
    let mut cx = cx();
    cx.grant(eval_remote_capability());
    let site = Arc::new(LocalEvalSite::new(ServerAddress::Local, codecs()));
    let mut transport = LocalTransport::new(ServerAddress::Local, site.clone());
    let frame = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        EvalRequest {
            expr: Expr::Bool(true),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            trace: false,
        },
    )
    .unwrap();
    transport.send_frame(&mut cx, frame).unwrap();
    let reply = transport.recv_frame(&mut cx, None).unwrap().unwrap();
    assert_eq!(reply.kind, FrameKind::Response);
}

#[test]
fn tcp_transport_uses_registry_loopback_only_when_requested() {
    #[derive(Clone)]
    struct LoopbackFallbackSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for LoopbackFallbackSite {
        fn site_kind(&self) -> &'static str {
            "loopback-fallback"
        }

        fn address(&self) -> &ServerAddress {
            &self.address
        }

        fn codecs(&self) -> &[Symbol] {
            &self.codecs
        }

        fn answer(
            &self,
            cx: &mut sim_kernel::Cx,
            frame: ServerFrame,
        ) -> sim_kernel::Result<ServerFrame> {
            let expr = eval_request_from_frame(cx, &frame)?.expr;
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::Response,
                &expr,
                frame.envelope.consistency,
                Vec::new(),
                false,
            )?;
            reply.correlate = frame.msg_id;
            Ok(reply)
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    let address = ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 65123,
    };
    register_endpoint(TransportEndpoint {
        address: address.clone(),
        site: Arc::new(LoopbackFallbackSite {
            address: address.clone(),
            codecs: codecs(),
        }),
    })
    .unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let err = connect_transport_site(&mut cx, address.clone(), codecs())
        .err()
        .unwrap();
    assert!(format!("{err}").contains("io"));

    let (client_site, selected) =
        connect_transport_site_with_loopback(&mut cx, address.clone(), codecs(), true).unwrap();
    assert_eq!(selected, Symbol::qualified("codec", "binary"));

    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("loopback".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::RemoteOnly,
            trace: false,
        },
    )
    .unwrap();
    let reply = client_site.answer(&mut cx, request).unwrap();
    assert_eq!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        Expr::String("loopback".to_owned())
    );

    unregister_endpoint(&address).unwrap();
}

#[test]
fn start_and_connect_capabilities_gate_network_surfaces() {
    let cx = cx();
    let tcp = ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 7171,
    };
    let unix = ServerAddress::Unix {
        path: PathBuf::from("/tmp/sim-say-capability.sock"),
    };
    let http = ServerAddress::Http {
        url: "http://127.0.0.1:7171".to_owned(),
    };

    let start_err = require_start_capabilities(&cx, &tcp).unwrap_err();
    assert!(format!("{start_err}").contains("network"));

    let connect_err = require_connect_capabilities(&cx, &tcp).unwrap_err();
    assert!(format!("{connect_err}").contains("network"));

    let unix_start_err = require_start_capabilities(&cx, &unix).unwrap_err();
    assert!(format!("{unix_start_err}").contains("network"));

    let unix_connect_err = require_connect_capabilities(&cx, &unix).unwrap_err();
    assert!(format!("{unix_connect_err}").contains("network"));

    let mut cx = cx;
    cx.grant_named("network");
    let http_err = require_start_capabilities(&cx, &http).unwrap_err();
    assert!(format!("{http_err}").contains("webhook-serve"));
}

#[cfg(not(unix))]
#[test]
fn unix_transports_report_not_available_on_non_unix_targets() {
    let address = ServerAddress::Unix {
        path: PathBuf::from("/tmp/sim-say-non-unix.sock"),
    };
    let err = super::super::open_server_transport(address.clone()).unwrap_err();
    assert!(format!("{err}").contains("unix sockets are not available"));

    let mut cx = cx();
    cx.grant_named("network");
    let err = connect_transport_site(&mut cx, address, codecs()).unwrap_err();
    assert!(format!("{err}").contains("unix sockets are not available"));
}

#[cfg(not(feature = "server-net-http"))]
#[test]
fn http_family_transports_fail_clearly_when_feature_is_disabled() {
    let address = ServerAddress::Http {
        url: "http://127.0.0.1:7171".to_owned(),
    };
    let err = super::super::open_server_transport(address.clone())
        .err()
        .unwrap();
    assert!(format!("{err}").contains("server-net-http"));

    let mut cx = cx();
    cx.grant_named("network");
    let err = connect_transport_site(&mut cx, address, codecs())
        .err()
        .unwrap();
    assert!(format!("{err}").contains("server-net-http"));
}

#[test]
fn wasm_transport_reuses_wasm_frame_limits() {
    let endpoint_site = Arc::new(LocalEvalSite::new(
        ServerAddress::Wasm {
            region: "shared-1".to_owned(),
        },
        codecs(),
    ));
    register_endpoint(TransportEndpoint {
        address: ServerAddress::Wasm {
            region: "shared-1".to_owned(),
        },
        site: endpoint_site.clone(),
    })
    .unwrap();

    let mut cx = cx();
    let request = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        EvalRequest {
            expr: Expr::Bool(true),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            trace: false,
        },
    )
    .unwrap();

    let bytes = encode_transport_frame(&request).unwrap();
    let endpoint_site: Arc<dyn EvalSite> = endpoint_site;
    let reply = route_frame_bytes(&mut cx, &endpoint_site, &bytes).unwrap();
    let decoded = decode_transport_frame(&reply).unwrap();
    assert_eq!(decoded.kind, FrameKind::Response);
}
