use super::{HttpRunnerRequest, post_json_with_tls_roots};
use rcgen::generate_simple_self_signed;
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use sim_lib_provider::Secret;
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    sync::Arc,
    thread,
    time::Duration,
};

#[test]
fn https_post_json_supports_tls_and_chunked_bodies() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let cert = generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let server_config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)
            .unwrap(),
    );
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let connection = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(connection, stream);
        let request = read_http_request(&mut stream);
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request_text.contains("Authorization: Bearer secret-token"));
        assert!(request.ends_with(br#"{"hello":"world"}"#));
        let body = "{\"ok\":true}";
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .unwrap();
        stream.flush().unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });
    let response = post_json_with_tls_roots(
        HttpRunnerRequest {
            runner_label: "runner/openai-compatible",
            endpoint: format!("https://127.0.0.1:{port}/v1"),
            path: "/chat/completions",
            headers: vec![
                (
                    "content-type".to_owned(),
                    Secret::new("application/json").unwrap(),
                ),
                (
                    "Authorization".to_owned(),
                    Secret::new("Bearer secret-token").unwrap(),
                ),
            ],
            timeout: Duration::from_secs(1),
            body: br#"{"hello":"world"}"#.to_vec(),
            max_response_bytes: 1024,
        },
        Some("secret-token"),
        Some(vec![cert_der]),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, body_bytes("{\"ok\":true}"));
    server.join().unwrap();
}

fn read_http_request(stream: &mut impl Read) -> Vec<u8> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut expected_len = None;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "HTTP request ended before its declared body");
        request.extend_from_slice(&chunk[..read]);
        if expected_len.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("HTTP request must declare content-length");
            expected_len = Some(header_end + 4 + content_len);
        }
        if expected_len.is_some_and(|expected| request.len() >= expected) {
            return request;
        }
    }
}

fn bind_loopback_listener() -> Option<TcpListener> {
    for _ in 0..3 {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => return Some(listener),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to bind loopback listener: {error}"),
        }
    }
    None
}

fn body_bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}
