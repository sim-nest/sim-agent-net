//! HTTP response body readers (chunked, content-length, read-to-eof).
//!
//! These do the socket-reading I/O around shared net-core body parsing;
//! `client.rs` picks one via `net_core::body_mode`.

use std::io::{BufReader, Read};

use sim_kernel::{Error, Result};

use super::{OptionalBodyChunkCallback, host_error};

pub(super) fn read_chunked_transfer_body(
    runner_label: &str,
    reader: &mut BufReader<&mut dyn Read>,
    max_response_bytes: usize,
    secret: Option<&str>,
    mut on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match sim_lib_net_core::decode_chunked(&encoded, max_response_bytes) {
            Ok(body) => {
                if let Some(callback) = on_body_chunk.as_deref_mut() {
                    callback(&body)?;
                }
                return Ok(body);
            }
            Err(sim_lib_net_core::NetError::TruncatedChunk) => {}
            Err(error) => return Err(map_chunked_error(runner_label, error)),
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|err| host_error(runner_label, err, secret))?;
        if read == 0 {
            return Err(Error::HostError("eof reading chunked response".to_owned()));
        }
        encoded.extend_from_slice(&buffer[..read]);
    }
}

fn map_chunked_error(runner_label: &str, error: sim_lib_net_core::NetError) -> Error {
    use sim_lib_net_core::NetError;
    match error {
        NetError::InvalidChunkSize(_) => Error::HostError("invalid chunk size".to_owned()),
        NetError::InvalidChunkDelimiter => Error::HostError("invalid chunked framing".to_owned()),
        NetError::OversizeBody(cap) => Error::Eval(format!(
            "{runner_label} response exceeded max output bytes {cap}"
        )),
        NetError::TruncatedChunk => Error::HostError("eof reading chunked response".to_owned()),
        _ => Error::HostError("invalid chunked framing".to_owned()),
    }
}

pub(super) fn read_content_length_body(
    runner_label: &str,
    reader: &mut BufReader<&mut dyn Read>,
    content_length: usize,
    secret: Option<&str>,
    mut on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(content_length);
    let mut remaining = content_length;
    let mut chunk = [0u8; 8192];
    while remaining > 0 {
        let wanted = remaining.min(chunk.len());
        reader
            .read_exact(&mut chunk[..wanted])
            .map_err(|err| host_error(runner_label, err, secret))?;
        body.extend_from_slice(&chunk[..wanted]);
        if let Some(callback) = on_body_chunk.as_deref_mut() {
            callback(&chunk[..wanted])?;
        }
        remaining -= wanted;
    }
    Ok(body)
}

pub(super) fn read_to_end_limited(
    runner_label: &str,
    reader: &mut BufReader<&mut dyn Read>,
    max_response_bytes: usize,
    secret: Option<&str>,
    mut on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|err| host_error(runner_label, err, secret))?;
        if read == 0 {
            return Ok(body);
        }
        if body.len().saturating_add(read) > max_response_bytes {
            return Err(Error::Eval(format!(
                "{runner_label} response exceeded max output bytes {max_response_bytes}"
            )));
        }
        body.extend_from_slice(&chunk[..read]);
        if let Some(callback) = on_body_chunk.as_deref_mut() {
            callback(&chunk[..read])?;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn chunked_transfer_uses_net_core_decoder_and_callback() {
        let mut input = Cursor::new(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n".to_vec());
        let mut reader = BufReader::new(&mut input as &mut dyn Read);
        let mut callbacks = Vec::new();
        let mut callback = |chunk: &[u8]| {
            callbacks.push(chunk.to_vec());
            Ok(())
        };

        let body =
            read_chunked_transfer_body("runner/test", &mut reader, 32, None, Some(&mut callback))
                .unwrap();

        assert_eq!(body, b"Wikipedia");
        assert_eq!(callbacks, vec![b"Wikipedia".to_vec()]);
    }

    #[test]
    fn chunked_transfer_maps_oversize_decoder_error() {
        let mut input = Cursor::new(b"5\r\nhello\r\n0\r\n\r\n".to_vec());
        let mut reader = BufReader::new(&mut input as &mut dyn Read);

        let err = read_chunked_transfer_body("runner/test", &mut reader, 4, None, None)
            .expect_err("oversize decoded body must fail")
            .to_string();

        assert!(
            err.contains("runner/test response exceeded max output bytes 4"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn chunked_transfer_maps_bad_framing_decoder_error() {
        let mut input = Cursor::new(b"1\r\na\n0\r\n\r\n".to_vec());
        let mut reader = BufReader::new(&mut input as &mut dyn Read);

        let err = read_chunked_transfer_body("runner/test", &mut reader, 32, None, None)
            .expect_err("bad chunk delimiter must fail")
            .to_string();

        assert!(
            err.contains("invalid chunked framing"),
            "unexpected error: {err}"
        );
    }
}
