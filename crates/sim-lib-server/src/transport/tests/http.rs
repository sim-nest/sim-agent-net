use std::sync::Arc;

use sim_kernel::{Consistency, Error, EvalRequest, Expr, NumberLiteral, ReadPolicy, Symbol};

use crate::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, ServerRuntime, ThreadMode,
    eval_request_from_frame, server_frame_from_request,
};

use super::super::{
    HttpServerTransport, SseServerTransport, WsServerTransport, connect_transport_site,
    run_accept_loop,
};
use super::support::{CollectingSink, codecs, cx};
use crate::transport::ServerTransport;

#[test]
fn http_transport_round_trips_a_frame_over_post_body() {
    #[derive(Clone)]
    struct HttpAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for HttpAnswerSite {
        fn site_kind(&self) -> &'static str {
            "http-answer"
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
                        canonical: "3".to_owned(),
                    }),
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "4".to_owned(),
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
                    canonical: "7".to_owned(),
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

    let transport = match HttpServerTransport::bind(ServerAddress::Http {
        url: "http://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("http bind failed: {error}"),
    };
    let address = transport.address().clone();
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(HttpAnswerSite {
        address: address.clone(),
        codecs: vec![
            Symbol::qualified("codec", "lisp"),
            Symbol::qualified("codec", "binary"),
        ],
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
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
                        canonical: "3".to_owned(),
                    }),
                    Expr::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "4".to_owned(),
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
            canonical: "7".to_owned()
        })
    );

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn http_transport_replies_with_error_frame_on_garbage_body() {
    use std::net::TcpStream;

    use crate::http::{HttpRequest, read_response, write_request};
    use crate::transport::decode_transport_frame;

    #[derive(Clone)]
    struct HttpAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for HttpAnswerSite {
        fn site_kind(&self) -> &'static str {
            "http-answer"
        }
        fn address(&self) -> &ServerAddress {
            &self.address
        }
        fn codecs(&self) -> &[Symbol] {
            &self.codecs
        }
        fn answer(
            &self,
            _cx: &mut sim_kernel::Cx,
            frame: ServerFrame,
        ) -> sim_kernel::Result<ServerFrame> {
            Ok(frame)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    let transport = match HttpServerTransport::bind(ServerAddress::Http {
        url: "http://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("http bind failed: {error}"),
    };
    let address = transport.address().clone();
    let ServerAddress::Http { url } = &address else {
        panic!("http transport bound to a non-http address");
    };
    let parsed = crate::http::parse_url(url, "http", "/sim/frame").unwrap();
    let (host, port, path) = (parsed.host.clone(), parsed.port, parsed.path.clone());

    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(HttpAnswerSite {
        address: address.clone(),
        codecs: vec![
            Symbol::qualified("codec", "lisp"),
            Symbol::qualified("codec", "binary"),
        ],
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut stream = TcpStream::connect((host.as_str(), port)).unwrap();
    write_request(
        &mut stream,
        &HttpRequest {
            method: "POST".to_owned(),
            path,
            headers: vec![
                ("Host".to_owned(), "sim-server".to_owned()),
                (
                    "Content-Type".to_owned(),
                    "application/sim-frame".to_owned(),
                ),
            ],
            body: b"not a valid transport frame".to_vec(),
        },
    )
    .unwrap();
    let response = read_response(&mut stream).unwrap();
    // The connection stayed up and produced a structured error frame, not a
    // dropped socket (regression for the `?`-drop on an undecodable body).
    assert_eq!(response.status, 200);
    let frame = decode_transport_frame(&response.body).unwrap();
    assert_eq!(frame.kind, FrameKind::Error);

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn http_transport_stream_fallback_returns_finite_chunk() {
    #[derive(Clone)]
    struct HttpStreamSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for HttpStreamSite {
        fn site_kind(&self) -> &'static str {
            "http-stream"
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
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::StreamChunk,
                &Expr::String("finite-http".to_owned()),
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

    let transport = match HttpServerTransport::bind(ServerAddress::Http {
        url: "http://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("http bind failed: {error}"),
    };
    let address = transport.address().clone();
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(HttpStreamSite {
        address: address.clone(),
        codecs: codecs(),
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("stream".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: true,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::RemoteOnly,
            trace: false,
        },
    )
    .unwrap();
    let mut sink = CollectingSink::default();
    client_site.stream(&mut cx, request, &mut sink).unwrap();

    assert_eq!(sink.chunks, vec![Expr::String("finite-http".to_owned())]);
    assert_eq!(sink.seen, vec![FrameKind::StreamChunk]);
    assert!(sink.ended);

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn sse_transport_streams_three_finite_chunks_then_end() {
    #[derive(Clone)]
    struct SseAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for SseAnswerSite {
        fn site_kind(&self) -> &'static str {
            "sse-answer"
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
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::Response,
                &Expr::String("fallback".to_owned()),
                frame.envelope.consistency,
                Vec::new(),
                false,
            )?;
            reply.correlate = frame.msg_id;
            Ok(reply)
        }
        fn stream(
            &self,
            cx: &mut sim_kernel::Cx,
            frame: ServerFrame,
            sink: &mut dyn crate::StreamSink,
        ) -> sim_kernel::Result<()> {
            sink.chunk(
                cx,
                ServerFrame::new(
                    frame.codec.clone(),
                    FrameKind::StreamStart,
                    frame.envelope.clone(),
                    Vec::new(),
                ),
            )?;
            for text in ["alpha", "beta", "gamma"] {
                let chunk = ServerFrame::from_expr(
                    cx,
                    frame.codec.clone(),
                    FrameKind::StreamChunk,
                    &Expr::String(text.to_owned()),
                    frame.envelope.consistency,
                    Vec::new(),
                    false,
                )?;
                sink.chunk(cx, chunk)?;
            }
            sink.chunk(
                cx,
                ServerFrame::new(
                    frame.codec,
                    FrameKind::StreamEnd,
                    frame.envelope,
                    Vec::new(),
                ),
            )?;
            sink.end(cx)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    let transport = match SseServerTransport::bind(ServerAddress::Sse {
        url: "http://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("sse bind failed: {error}"),
    };
    let address = transport.address().clone();
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(SseAnswerSite {
        address: address.clone(),
        codecs: codecs(),
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("stream".to_owned()),
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
    let mut sink = CollectingSink::default();
    client_site.stream(&mut cx, request, &mut sink).unwrap();
    assert_eq!(
        sink.chunks,
        vec![
            Expr::String("alpha".to_owned()),
            Expr::String("beta".to_owned()),
            Expr::String("gamma".to_owned())
        ]
    );
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

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn websocket_transport_round_trips_one_frame() {
    #[derive(Clone)]
    struct WsAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for WsAnswerSite {
        fn site_kind(&self) -> &'static str {
            "ws-answer"
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
            assert_eq!(frame.codec, Symbol::qualified("codec", "lisp"));
            let expr = eval_request_from_frame(cx, &frame)?.expr;
            assert_eq!(expr, Expr::String("socket".to_owned()));
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::Response,
                &Expr::String("ok".to_owned()),
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

    let transport = match WsServerTransport::bind(ServerAddress::Ws {
        url: "ws://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("ws bind failed: {error}"),
    };
    let address = transport.address().clone();
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(WsAnswerSite {
        address: address.clone(),
        codecs: codecs(),
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, _selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "lisp"),
        EvalRequest {
            expr: Expr::String("socket".to_owned()),
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
    assert_eq!(reply.codec, Symbol::qualified("codec", "lisp"));
    assert_eq!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        Expr::String("ok".to_owned())
    );

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn websocket_transport_stream_fallback_returns_finite_chunk() {
    #[derive(Clone)]
    struct WsStreamSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for WsStreamSite {
        fn site_kind(&self) -> &'static str {
            "ws-stream"
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
            let mut reply = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::StreamChunk,
                &Expr::String("finite-ws".to_owned()),
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

    let transport = match WsServerTransport::bind(ServerAddress::Ws {
        url: "ws://127.0.0.1:0".to_owned(),
    }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("ws bind failed: {error}"),
    };
    let address = transport.address().clone();
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(WsStreamSite {
        address: address.clone(),
        codecs: codecs(),
    });
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut cx = cx();
    cx.grant_named("network");
    let (client_site, selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
    let request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("stream".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: true,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::RemoteOnly,
            trace: false,
        },
    )
    .unwrap();
    let mut sink = CollectingSink::default();
    client_site.stream(&mut cx, request, &mut sink).unwrap();

    assert_eq!(sink.chunks, vec![Expr::String("finite-ws".to_owned())]);
    assert_eq!(sink.seen, vec![FrameKind::StreamChunk]);
    assert!(sink.ended);

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}
