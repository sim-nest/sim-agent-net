use std::io::Cursor;

use super::{
    ParsedUrl, base64_decode, base64_encode, parse_url, read_request, read_sse_event,
    websocket_accept_value,
};

// Pins host/port/path resolution for every transport call site: http/https/ws
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

// A single-`data:` event is the common SSE case and keeps its payload exactly.
#[test]
fn sse_single_data_event_is_unchanged() {
    let input = "event: chunk\r\ndata: payload\r\n\r\n";
    let mut reader = Cursor::new(input.as_bytes());
    let event = read_sse_event(&mut reader).unwrap().unwrap();
    assert_eq!(event, ("chunk".to_owned(), "payload".to_owned()));
}

// Multiple `data:` lines in one record fold with `\n`, matching the SSE
// behavior of `sim_lib_net_core::SseDecoder`.
#[test]
fn sse_folds_multiple_data_lines() {
    let input = "event: message\r\ndata: one\r\ndata: two\r\n\r\n";
    let mut reader = Cursor::new(input.as_bytes());
    let event = read_sse_event(&mut reader).unwrap().unwrap();
    assert_eq!(event, ("message".to_owned(), "one\ntwo".to_owned()));
}

// The transport's own wire form (an "end" event with empty data, exactly as
// `SseStreamSink::write_event` emits it) round-trips through the net-core
// decoder.
#[test]
fn sse_round_trips_transport_end_event() {
    let input = "event: end\r\ndata: \r\n\r\n";
    let mut reader = Cursor::new(input.as_bytes());
    let event = read_sse_event(&mut reader).unwrap().unwrap();
    assert_eq!(event, ("end".to_owned(), String::new()));
}

// EOF with nothing accumulated yields no event.
#[test]
fn sse_eof_without_record_is_none() {
    let mut reader = Cursor::new(b"".as_slice());
    assert!(read_sse_event(&mut reader).unwrap().is_none());
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
