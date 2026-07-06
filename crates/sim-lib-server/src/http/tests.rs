use std::io::Cursor;

use super::{
    ParsedUrl, base64_decode, base64_encode, parse_url, read_request, websocket_accept_value,
};

// OVERLAP9.01: pin the host/port/path resolution for every transport call site
// after routing `parse_url` through `sim_lib_net_core`. Covers http/https/ws
// bind + connect, portless authority (ws -> :80), and trailing-slash paths.
#[test]
fn parse_url_matrix_matches_transport_call_sites() {
    // http bind/connect with an explicit port and a path.
    assert_eq!(
        parse_url("http://host:8080/sim/frame", "http", "/sim/frame").unwrap(),
        ParsedUrl {
            scheme: "http".to_owned(),
            host: "host".to_owned(),
            port: 8080,
            path: "/sim/frame".to_owned(),
        }
    );
    // http portless authority defaults to :80 and substitutes the default path.
    assert_eq!(
        parse_url("http://host", "http", "/sim/frame").unwrap(),
        ParsedUrl {
            scheme: "http".to_owned(),
            host: "host".to_owned(),
            port: 80,
            path: "/sim/frame".to_owned(),
        }
    );
    // https with an explicit port (transports never pass a portless https URL).
    assert_eq!(
        parse_url("https://host:8443/v1/chat", "https", "/sim/frame").unwrap(),
        ParsedUrl {
            scheme: "https".to_owned(),
            host: "host".to_owned(),
            port: 8443,
            path: "/v1/chat".to_owned(),
        }
    );
    // ws portless authority defaults to :80 and substitutes the default path.
    assert_eq!(
        parse_url("ws://host", "ws", "/sim/ws").unwrap(),
        ParsedUrl {
            scheme: "ws".to_owned(),
            host: "host".to_owned(),
            port: 80,
            path: "/sim/ws".to_owned(),
        }
    );
    // ws with an explicit port and path.
    assert_eq!(
        parse_url("ws://host:9001/sim/ws", "ws", "/sim/ws").unwrap(),
        ParsedUrl {
            scheme: "ws".to_owned(),
            host: "host".to_owned(),
            port: 9001,
            path: "/sim/ws".to_owned(),
        }
    );
    // A caller's trailing slash is preserved, not trimmed.
    assert_eq!(
        parse_url("http://host/a/b/", "http", "/sim/frame")
            .unwrap()
            .path,
        "/a/b/"
    );
    // Scheme mismatch is rejected.
    assert!(parse_url("http://host/x", "ws", "/sim/ws").is_err());
}

#[test]
fn base64_round_trips() {
    let encoded = base64_encode(b"hello");
    assert_eq!(base64_decode(&encoded).unwrap(), b"hello");
}

#[test]
fn websocket_accept_matches_reference() {
    let accept = websocket_accept_value("dGhlIHNhbXBsZSBub25jZQ==");
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn oversized_content_length_is_rejected_before_body_allocation() {
    let input = format!(
        "POST /sim/frame HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        usize::MAX
    );
    let err = read_request(&mut Cursor::new(input.into_bytes())).unwrap_err();
    assert!(format!("{err}").contains("content-length exceeds"));
}
