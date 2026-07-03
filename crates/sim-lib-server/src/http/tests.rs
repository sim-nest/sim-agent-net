use std::io::Cursor;

use super::{base64_decode, base64_encode, read_request, websocket_accept_value};

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
