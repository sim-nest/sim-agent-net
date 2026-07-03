use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use sim_kernel::{
    CapabilityName, Consistency, Error, EvalRequest, Expr, NumberLiteral, ReadPolicy, Symbol,
};

use crate::{
    EvalSite, FrameKind, LocalEvalSite, ServerAddress, ServerFrame, ServerRuntime, ThreadMode,
    eval_reply_from_frame, eval_request_from_frame, server_frame_from_request,
};

use super::super::{TcpServerTransport, connect_transport_site, run_accept_loop};
use super::support::{codecs, cx};

#[test]
fn lisp_request_frame_round_trips_through_payload_decode() {
    let mut cx = cx();
    let request = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "lisp"),
        EvalRequest {
            expr: Expr::String("per-frame".to_owned()),
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

    let encoded = String::from_utf8(request.payload.clone()).unwrap();
    assert_eq!(
        encoded,
        "(expr:map [answer-limit nil] [consistency remote-only] [deadline nil] [expr \"per-frame\"] [mode eval] [requires ()] [result-shape nil] [stream false] [stream-buffer nil] [trace false])"
    );

    let decoded_from_text = sim_codec::decode_with_codec(
        &mut cx,
        &Symbol::qualified("codec", "lisp"),
        sim_codec::Input::Text(encoded.clone()),
        ReadPolicy::default(),
    )
    .unwrap();
    assert!(matches!(decoded_from_text, Expr::Map(_)));

    let decoded = request.decode_expr(&mut cx, ReadPolicy::default()).unwrap();
    assert!(matches!(decoded, Expr::Map(_)));
}

#[test]
fn tcp_transport_negotiates_and_answers_over_a_real_socket() {
    #[derive(Clone)]
    struct SocketAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for SocketAnswerSite {
        fn site_kind(&self) -> &'static str {
            "socket-answer"
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
            let expected = Expr::Call {
                operator: Box::new(Expr::Symbol(Symbol::new("+"))),
                args: vec![
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "1".to_owned(),
                    }),
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "2".to_owned(),
                    }),
                ],
            };
            assert_eq!(expr, expected);
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::Response,
                &Expr::Number(NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "3".to_owned(),
                }),
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

    let transport = match TcpServerTransport::bind(ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 0,
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("tcp bind failed: {error}"),
    };
    let port = transport.local_port().unwrap();
    let site = Arc::new(SocketAnswerSite {
        address: ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs: vec![
            Symbol::qualified("codec", "lisp"),
            Symbol::qualified("codec", "binary"),
        ],
    });
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(
        &mut cx,
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        vec![
            Symbol::qualified("codec", "binary"),
            Symbol::qualified("codec", "json"),
        ],
    )
    .unwrap();
    assert_eq!(selected, Symbol::qualified("codec", "binary"));

    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::Call {
                operator: Box::new(Expr::Symbol(Symbol::new("+"))),
                args: vec![
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "1".to_owned(),
                    }),
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "2".to_owned(),
                    }),
                ],
            },
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
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "3".to_owned()
        })
    );

    drop(client_site);
    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn tcp_transport_enforces_required_capabilities_across_wire() {
    let transport = match TcpServerTransport::bind(ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 0,
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("tcp bind failed: {error}"),
    };
    let port = transport.local_port().unwrap();
    let site = Arc::new(LocalEvalSite::new(
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs(),
    ));
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(
        &mut cx,
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs(),
    )
    .unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("guarded".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: vec![CapabilityName::new("wire.cap")],
            deadline: None,
            consistency: Consistency::RemoteOnly,
            trace: false,
        },
    )
    .unwrap();
    let reply = client_site.answer(&mut cx, request).unwrap();
    assert_eq!(reply.kind, FrameKind::Error);
    assert!(matches!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        Expr::String(message) if message.contains("wire.cap")
    ));

    drop(client_site);
    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn tcp_accept_loop_keeps_accepting_while_another_client_stays_connected() {
    let transport = match TcpServerTransport::bind(ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 0,
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("tcp bind failed: {error}"),
    };
    let port = transport.local_port().unwrap();
    let site = Arc::new(LocalEvalSite::new(
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs(),
    ));
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut first_cx = cx();
    first_cx.grant_named("network");
    let (_first_site, first_codec) = connect_transport_site(
        &mut first_cx,
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs(),
    )
    .unwrap();
    assert_eq!(first_codec, Symbol::qualified("codec", "binary"));

    let (done_tx, done_rx) = mpsc::channel();
    let request_port = port;
    std::thread::spawn(move || {
        let mut second_cx = cx();
        second_cx.grant_named("network");
        let outcome = connect_transport_site(
            &mut second_cx,
            ServerAddress::Tcp {
                host: "127.0.0.1".to_owned(),
                port: request_port,
            },
            codecs(),
        )
        .and_then(|(client_site, selected)| {
            let request = server_frame_from_request(
                &mut second_cx,
                &selected,
                EvalRequest {
                    expr: Expr::String("concurrent".to_owned()),
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
            )?;
            client_site.answer(&mut second_cx, request)
        });
        let _ = done_tx.send(outcome);
    });

    let reply = done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(
        eval_reply_from_frame(&mut first_cx, &reply)
            .unwrap()
            .value
            .object()
            .as_expr(&mut first_cx)
            .unwrap(),
        Expr::String("concurrent".to_owned())
    );

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}
