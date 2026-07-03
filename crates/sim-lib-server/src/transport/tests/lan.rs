use std::{net::TcpListener, sync::Arc};

use sim_kernel::{Consistency, EvalRequest, Expr, ReadPolicy, Symbol};

use crate::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_request,
};

use super::super::{connect_transport_site_with_loopback, register_loopback_transport_endpoint};
use super::support::{codecs, cx};

#[derive(Clone)]
struct LanPeerSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
}

impl EvalSite for LanPeerSite {
    fn site_kind(&self) -> &'static str {
        "lan-peer"
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

#[test]
fn lan_peer_loopback_routes_fake_distributed_node() {
    let address = unused_loopback_tcp_address();
    let registration = register_loopback_transport_endpoint(
        address.clone(),
        Arc::new(LanPeerSite {
            address: address.clone(),
            codecs: codecs(),
        }),
    )
    .unwrap();
    assert_eq!(registration.address(), &address);

    let mut cx = cx();
    cx.grant_named("network");
    let (client, codec) =
        connect_transport_site_with_loopback(&mut cx, address.clone(), codecs(), true).unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &codec,
        EvalRequest {
            expr: lan_peer_payload(),
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

    let reply = client.answer(&mut cx, request).unwrap();
    assert_eq!(reply.kind, FrameKind::Response);
    assert_eq!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        lan_peer_payload()
    );
    registration.close().unwrap();
}

fn lan_peer_payload() -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("site")),
            Expr::Symbol(Symbol::qualified("stream/site", "lan-peer")),
        ),
        (
            Expr::Symbol(Symbol::new("mode")),
            Expr::Symbol(Symbol::qualified("stream/lan-mode", "jitter-buffered")),
        ),
    ])
}

fn unused_loopback_tcp_address() -> ServerAddress {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port,
    }
}
