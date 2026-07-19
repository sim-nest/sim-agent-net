use std::io::{BufRead, Read, Write};

use sim_kernel::{Error, Result};

use crate::transport::MAX_TRANSPORT_FRAME_BYTES;

use super::core::{HttpRequest, HttpResponse, header_value};

const MAX_SSE_LINE_BYTES: usize = 64 * 1024;

pub(crate) fn read_request<R: Read>(reader: &mut R) -> Result<Option<HttpRequest>> {
    let head = match read_http_head(reader)? {
        Some(head) => head,
        None => return Ok(None),
    };
    let mut lines = parse_http_head(&head)?;
    let request_line = lines
        .next()
        .ok_or_else(|| Error::HostError("http request missing request line".to_owned()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| Error::HostError("http request missing method".to_owned()))?;
    let path = parts
        .next()
        .ok_or_else(|| Error::HostError("http request missing path".to_owned()))?;
    let _version = parts
        .next()
        .ok_or_else(|| Error::HostError("http request missing version".to_owned()))?;
    let headers = parse_headers(lines)?;
    let body_len = content_length(&headers)?;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).map_err(io_to_host)?;
    Ok(Some(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body,
    }))
}

pub(crate) fn write_request<W: Write>(writer: &mut W, req: &HttpRequest) -> Result<()> {
    write!(writer, "{} {} HTTP/1.1\r\n", req.method, req.path).map_err(io_to_host)?;
    write_headers(writer, &req.headers, req.body.len())?;
    writer.write_all(&req.body).map_err(io_to_host)?;
    writer.flush().map_err(io_to_host)
}

pub(crate) fn read_response<R: Read>(reader: &mut R) -> Result<HttpResponse> {
    let head = read_http_head(reader)?
        .ok_or_else(|| Error::HostError("http response closed before headers".to_owned()))?;
    // Response heads (status line + headers) parse through the shared net-core
    // primitive; the server keeps only its error mapping and body-length cap.
    let text = std::str::from_utf8(&head)
        .map_err(|_| Error::HostError("http headers are not valid utf-8".to_owned()))?;
    let parsed = sim_lib_net_core::parse_http_head(text)
        .map_err(|error| Error::HostError(format!("invalid http response: {error}")))?;
    let body_len = content_length(&parsed.headers)?;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).map_err(io_to_host)?;
    Ok(HttpResponse {
        status: parsed.status,
        headers: parsed.headers,
        body,
    })
}

pub(crate) fn write_response<W: Write>(writer: &mut W, res: &HttpResponse) -> Result<()> {
    write!(
        writer,
        "HTTP/1.1 {} {}\r\n",
        res.status,
        status_text(res.status)
    )
    .map_err(io_to_host)?;
    write_headers(writer, &res.headers, res.body.len())?;
    writer.write_all(&res.body).map_err(io_to_host)?;
    writer.flush().map_err(io_to_host)
}

// SSE record decoding defers to `sim_lib_net_core::SseDecoder`. That decoder
// folds multiple `data:` lines of one record with `\n` (the SSE spec); this
// server's earlier local reader kept only the LAST `data:` line. Adopting the
// fold is a deliberate, tested behavior change (see the crate tests). The
// `(event, data)` tuple contract is preserved for the transport consumers.
pub(crate) fn read_sse_event<R: BufRead>(reader: &mut R) -> Result<Option<(String, String)>> {
    let mut decoder = sim_lib_net_core::SseDecoder::new();
    let mut data_bytes = 0usize;
    loop {
        let mut line = String::new();
        match sim_lib_net_core::read_capped_line(reader, &mut line, MAX_SSE_LINE_BYTES)
            .map_err(io_to_host)?
        {
            sim_lib_net_core::CapOutcome::Eof => {
                // Peer closed: emit any record accumulated without a final blank line.
                return Ok(decoder.flush().map(sse_event_tuple));
            }
            sim_lib_net_core::CapOutcome::TooLarge => {
                return Err(Error::HostError(format!(
                    "sse line exceeds size limit of {MAX_SSE_LINE_BYTES} bytes"
                )));
            }
            sim_lib_net_core::CapOutcome::Line => {}
        }
        let line = line.trim_end_matches(['\r', '\n']);
        update_sse_data_bound(line, &mut data_bytes)?;
        if let Some(event) = decoder.push_line(line) {
            return Ok(Some(sse_event_tuple(event)));
        }
        if line.is_empty() {
            data_bytes = 0;
        }
    }
}

fn sse_event_tuple(event: sim_lib_net_core::SseEvent) -> (String, String) {
    (event.event.unwrap_or_default(), event.data)
}

fn update_sse_data_bound(line: &str, data_bytes: &mut usize) -> Result<()> {
    if line.is_empty() {
        *data_bytes = 0;
        return Ok(());
    }
    if line.starts_with(':') {
        return Ok(());
    }
    let (field, value) = match line.split_once(':') {
        Some((field, rest)) => (field, rest.strip_prefix(' ').unwrap_or(rest)),
        None => (line, ""),
    };
    if field != "data" {
        return Ok(());
    }
    let separator = usize::from(*data_bytes != 0);
    let next = (*data_bytes)
        .saturating_add(separator)
        .saturating_add(value.len());
    if next > MAX_SSE_LINE_BYTES {
        return Err(Error::HostError(format!(
            "sse event data exceeds size limit of {MAX_SSE_LINE_BYTES} bytes"
        )));
    }
    *data_bytes = next;
    Ok(())
}

fn read_http_head<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    // 64 KiB HTTP head cap (caller policy); the read-until-CRLFCRLF framing is
    // the shared net-core primitive. The outcome mapping keeps this server's
    // local error text and None-on-empty-peer behavior unchanged.
    match sim_lib_net_core::read_head_until_double_crlf(reader, 64 * 1024).map_err(io_to_host)? {
        sim_lib_net_core::HeadOutcome::Head(head) => Ok(Some(head)),
        sim_lib_net_core::HeadOutcome::Eof => Ok(None),
        sim_lib_net_core::HeadOutcome::Truncated(_) => {
            Err(Error::HostError("truncated http headers".to_owned()))
        }
        sim_lib_net_core::HeadOutcome::TooLarge => Err(Error::HostError(
            "http headers exceed size limit".to_owned(),
        )),
    }
}

// Request heads keep a local line splitter: `sim_lib_net_core::parse_http_head`
// parses a response status line (`version status reason`) and cannot read an
// HTTP request line (`method path version`). Response heads use the net-core
// primitive directly (see `read_response`).
fn parse_http_head(head: &[u8]) -> Result<std::vec::IntoIter<String>> {
    let text = std::str::from_utf8(head)
        .map_err(|_| Error::HostError("http headers are not valid utf-8".to_owned()))?;
    Ok(text
        .trim_end_matches("\r\n\r\n")
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
        .into_iter())
}

fn parse_headers(lines: std::vec::IntoIter<String>) -> Result<Vec<(String, String)>> {
    lines
        .map(|line| {
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| Error::HostError("invalid http header line".to_owned()))?;
            Ok((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn content_length(headers: &[(String, String)]) -> Result<usize> {
    if let Some(value) = header_value(headers, "Transfer-Encoding")
        && value.eq_ignore_ascii_case("chunked")
    {
        return Err(Error::HostError(
            "chunked transfer encoding is not supported".to_owned(),
        ));
    }
    let Some(value) = header_value(headers, "Content-Length") else {
        return Ok(0);
    };
    let len = value
        .parse::<usize>()
        .map_err(|_| Error::HostError("invalid content-length".to_owned()))?;
    if len > MAX_TRANSPORT_FRAME_BYTES {
        return Err(Error::HostError(
            "http content-length exceeds size limit".to_owned(),
        ));
    }
    Ok(len)
}

fn write_headers<W: Write>(
    writer: &mut W,
    headers: &[(String, String)],
    body_len: usize,
) -> Result<()> {
    let mut wrote_length = false;
    let mut wrote_connection = false;
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("Content-Length") {
            wrote_length = true;
        }
        if key.eq_ignore_ascii_case("Connection") {
            wrote_connection = true;
        }
        write!(writer, "{key}: {value}\r\n").map_err(io_to_host)?;
    }
    if !wrote_length {
        write!(writer, "Content-Length: {body_len}\r\n").map_err(io_to_host)?;
    }
    if !wrote_connection {
        writer
            .write_all(b"Connection: keep-alive\r\n")
            .map_err(io_to_host)?;
    }
    writer.write_all(b"\r\n").map_err(io_to_host)
}

fn status_text(status: u16) -> &'static str {
    match status {
        101 => "Switching Protocols",
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        426 => "Upgrade Required",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn io_to_host(error: std::io::Error) -> Error {
    Error::HostError(format!("io {:?}: {}", error.kind(), error))
}
