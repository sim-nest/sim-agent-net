use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

use sim_kernel::{Consistency, Error, EvalRequest, Expr, ReadPolicy, Symbol};

use crate::{
    Connection, FrameKind, LocalEvalSite, ServerAddress, ServerFrame, ServerRuntime, ThreadMode,
    decode_transport_frame, encode_transport_frame, server_frame_from_request,
};

use super::super::{TcpServerTransport, connect_transport_site, run_accept_loop};
use super::support::{codecs, cx};

#[test]
fn tcp_socket_path_rejects_truncated_prefix() {
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

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.write_all(&[0, 1, 2]).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut reply = [0u8; 1];
    assert_eq!(client.read(&mut reply).unwrap_or(0), 0);

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn server_request_timeout_returns_clear_error_for_silent_tcp_peer() {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("tcp bind failed: {error}"),
    };
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let negotiate = read_raw_transport_frame(&mut stream);
        assert!(matches!(negotiate.kind, FrameKind::Negotiate { .. }));
        let reply = ServerFrame {
            version: negotiate.version,
            codec: Symbol::qualified("codec", "binary"),
            msg_id: None,
            correlate: negotiate.msg_id,
            kind: FrameKind::Negotiate {
                codecs: vec![Symbol::qualified("codec", "binary")],
            },
            envelope: Default::default(),
            payload: Vec::new(),
        };
        let bytes = encode_transport_frame(&reply).unwrap();
        stream.write_all(&bytes).unwrap();

        let _request = read_raw_transport_frame(&mut stream);
        std::thread::sleep(Duration::from_millis(100));
    });

    let mut cx = cx();
    cx.grant_named("network");
    cx.grant(sim_kernel::eval_remote_capability());
    let (site, selected) = connect_transport_site(
        &mut cx,
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        codecs(),
    )
    .unwrap();
    let connection = Connection::new(
        ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port,
        },
        selected,
        codecs(),
        site,
    )
    .unwrap();

    let error = connection
        .request(
            &mut cx,
            Expr::String("timeout".to_owned()),
            Some(Duration::from_millis(25)),
            Vec::new(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Eval(message) if message == "request timed out after 25ms"
    ));

    peer.join().unwrap();
}

#[test]
fn reply_codec_hint_switches_socket_reply_codec() {
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
        vec![Symbol::qualified("codec", "binary")],
    )
    .unwrap();
    assert_eq!(selected, Symbol::qualified("codec", "binary"));

    let mut request = server_frame_from_request(
        &mut cx,
        &selected,
        EvalRequest {
            expr: Expr::String("reply-codec".to_owned()),
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
    request.envelope.reply_codec_hint = Some(Symbol::qualified("codec", "lisp"));
    let reply = client_site.answer(&mut cx, request).unwrap();
    assert_eq!(reply.codec, Symbol::qualified("codec", "lisp"));
    assert_eq!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("value")),
                Expr::String("reply-codec".to_owned())
            ),
            (
                Expr::Symbol(Symbol::new("diagnostics")),
                Expr::List(Vec::new())
            ),
            (Expr::Symbol(Symbol::new("trace")), Expr::Nil),
        ])
    );

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn tcp_socket_path_rejects_partial_body_then_eof_cleanly() {
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
    let frame = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        EvalRequest {
            expr: Expr::String("partial".to_owned()),
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
    let bytes = encode_transport_frame(&frame).unwrap();
    let split = bytes.len().saturating_sub(2);

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.write_all(&bytes[..split]).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut reply = [0u8; 1];
    assert_eq!(client.read(&mut reply).unwrap_or(0), 0);

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

#[test]
fn max_inflight_overflow_returns_error_frame() {
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
    let runtime = Arc::new(ServerRuntime::new(transport, cx(), ThreadMode::Spawn, 0));
    let handle = std::thread::spawn({
        let runtime = runtime.clone();
        let site = site.clone();
        move || run_accept_loop(runtime, site)
    });
    runtime.set_accept_thread(handle).unwrap();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let negotiate = ServerFrame {
        version: 1,
        codec: Symbol::qualified("codec", "binary"),
        msg_id: Some(1),
        correlate: None,
        kind: FrameKind::Negotiate {
            codecs: vec![Symbol::qualified("codec", "binary")],
        },
        envelope: Default::default(),
        payload: Vec::new(),
    };
    stream
        .write_all(&encode_transport_frame(&negotiate).unwrap())
        .unwrap();
    let reply = read_raw_transport_frame(&mut stream);
    assert!(matches!(reply.kind, FrameKind::Negotiate { .. }));

    let request = ServerFrame {
        version: 1,
        codec: Symbol::qualified("codec", "binary"),
        msg_id: Some(2),
        correlate: None,
        kind: FrameKind::Request,
        envelope: Default::default(),
        payload: server_frame_from_request(
            &mut cx(),
            &Symbol::qualified("codec", "binary"),
            EvalRequest {
                expr: Expr::String("overflow".to_owned()),
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
        .unwrap()
        .payload,
    };
    stream
        .write_all(&encode_transport_frame(&request).unwrap())
        .unwrap();
    let error = read_raw_transport_frame(&mut stream);
    assert_eq!(error.kind, FrameKind::Error);
    assert_eq!(
        error.decode_expr(&mut cx(), ReadPolicy::default()).unwrap(),
        Expr::String("evaluation error: connection max-inflight 0 exceeded".to_owned())
    );

    runtime.begin_stop();
    runtime.join_accept_thread().unwrap();
    runtime.join_worker_threads().unwrap();
}

fn read_raw_transport_frame(stream: &mut TcpStream) -> ServerFrame {
    let mut prefix = [0u8; 8];
    stream.read_exact(&mut prefix).unwrap();
    let header_len = u32::from_be_bytes(prefix[..4].try_into().unwrap()) as usize;
    let payload_len = u32::from_be_bytes(prefix[4..].try_into().unwrap()) as usize;
    let mut body = vec![0u8; header_len + payload_len];
    stream.read_exact(&mut body).unwrap();
    let mut frame = prefix.to_vec();
    frame.extend_from_slice(&body);
    decode_transport_frame(&frame).unwrap()
}
