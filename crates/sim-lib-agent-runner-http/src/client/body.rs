//! HTTP response body readers (chunked, content-length, read-to-eof).
//!
//! These do the socket-reading I/O that `sim-lib-net-core` deliberately leaves
//! to the transport; `client.rs` picks one via `net_core::body_mode`.

use std::io::{BufRead, BufReader, Read};

use sim_kernel::{Error, Result};

use super::{OptionalBodyChunkCallback, host_error};

pub(super) fn read_chunked_body(
    runner_label: &str,
    reader: &mut BufReader<&mut dyn Read>,
    max_response_bytes: usize,
    secret: Option<&str>,
    mut on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|err| host_error(runner_label, err, secret))?;
        if size_line.is_empty() {
            return Err(Error::HostError("eof reading chunked response".to_owned()));
        }
        let size_text = size_line
            .trim_end_matches(['\r', '\n'])
            .split(';')
            .next()
            .unwrap_or_default();
        let chunk_size = usize::from_str_radix(size_text, 16)
            .map_err(|_| Error::HostError("invalid chunk size".to_owned()))?;
        if chunk_size == 0 {
            loop {
                let mut trailer_line = String::new();
                reader
                    .read_line(&mut trailer_line)
                    .map_err(|err| host_error(runner_label, err, secret))?;
                if trailer_line == "\r\n" || trailer_line.is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len().saturating_add(chunk_size) > max_response_bytes {
            return Err(Error::Eval(format!(
                "{runner_label} response exceeded max output bytes {max_response_bytes}"
            )));
        }
        let start = body.len();
        body.resize(start + chunk_size, 0);
        reader
            .read_exact(&mut body[start..])
            .map_err(|err| host_error(runner_label, err, secret))?;
        if let Some(callback) = on_body_chunk.as_deref_mut() {
            callback(&body[start..])?;
        }
        let mut chunk_ending = [0u8; 2];
        reader
            .read_exact(&mut chunk_ending)
            .map_err(|err| host_error(runner_label, err, secret))?;
        if chunk_ending != *b"\r\n" {
            return Err(Error::HostError("invalid chunked framing".to_owned()));
        }
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
