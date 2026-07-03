use std::sync::Arc;

use sim_kernel::{Consistency, Error, EvalRequest, Expr, NumberLiteral, ReadPolicy, Symbol};

use crate::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, ServerRuntime, ThreadMode,
    eval_request_from_frame, server_frame_from_request,
};

use super::super::{ServerTransport, UnixServerTransport, connect_transport_site, run_accept_loop};
use super::support::{cx, unix_socket_path};

#[test]
fn unix_transport_rebinds_after_stale_socket_file() {
    use std::os::unix::net::UnixListener;

    let path = unix_socket_path("stale");
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("unix bind failed: {error}"),
    };
    drop(listener);
    assert!(path.exists());

    let transport = UnixServerTransport::bind(ServerAddress::Unix { path: path.clone() }).unwrap();
    assert!(path.exists());
    let mut cx = cx();
    transport.shutdown(&mut cx).unwrap();
    assert!(!path.exists());
}

#[test]
fn unix_transport_negotiates_and_answers_over_a_real_socket() {
    #[derive(Clone)]
    struct UnixAnswerSite {
        address: ServerAddress,
        codecs: Vec<Symbol>,
    }

    impl EvalSite for UnixAnswerSite {
        fn site_kind(&self) -> &'static str {
            "unix-answer"
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
                        canonical: "2".to_owned(),
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
                    canonical: "4".to_owned(),
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

    let path = unix_socket_path("cross-socket");
    let transport = match UnixServerTransport::bind(ServerAddress::Unix { path: path.clone() }) {
        Ok(transport) => Arc::new(transport),
        Err(Error::HostError(message)) if message.contains("PermissionDenied") => return,
        Err(error) => panic!("unix bind failed: {error}"),
    };
    let runtime = Arc::new(ServerRuntime::new(
        transport,
        cx(),
        ThreadMode::Spawn,
        crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES,
    ));
    let site = Arc::new(UnixAnswerSite {
        address: ServerAddress::Unix { path: path.clone() },
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
    let (client_site, selected) = connect_transport_site(
        &mut cx,
        ServerAddress::Unix { path: path.clone() },
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
                        canonical: "2".to_owned(),
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
            canonical: "4".to_owned()
        })
    );

    drop(client_site);
    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
    runtime
        .with_cx(|cx| runtime.transport().shutdown(cx))
        .unwrap();
    assert!(!path.exists());
}
