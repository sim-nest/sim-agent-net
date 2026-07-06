use std::io::{BufRead, Read, Write};

use sim_kernel::{Error, Result};

use crate::transport::MAX_TRANSPORT_FRAME_BYTES;

use super::core::{HttpRequest, HttpResponse, header_value};

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
    let mut lines = parse_http_head(&head)?;
    let status_line = lines
        .next()
        .ok_or_else(|| Error::HostError("http response missing status line".to_owned()))?;
    let mut parts = status_line.split_whitespace();
    let _version = parts
        .next()
        .ok_or_else(|| Error::HostError("http response missing version".to_owned()))?;
    let status = parts
        .next()
        .ok_or_else(|| Error::HostError("http response missing status".to_owned()))?
        .parse::<u16>()
        .map_err(|_| Error::HostError("invalid http response status".to_owned()))?;
    let headers = parse_headers(lines)?;
    let body_len = content_length(&headers)?;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).map_err(io_to_host)?;
    Ok(HttpResponse {
        status,
        headers,
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

// NOTE (OVERLAP8.06): intentionally NOT routed through
// `sim_lib_net_core::SseDecoder`. This reader keeps only the LAST `data:` line of
// an event, whereas `SseDecoder` folds multiple `data:` lines with `\n` per the
// SSE spec. Adopting the folding decoder is a wire-visible behavior change for
// this transport, so it is deferred until a test proves the multi-`data:` form
// is intended here.
pub(crate) fn read_sse_event<R: BufRead>(reader: &mut R) -> Result<Option<(String, String)>> {
    let mut event = None;
    let mut data = String::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(io_to_host)?;
        if read == 0 {
            if event.is_none() && data.is_empty() {
                return Ok(None);
            }
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data = value.trim().to_owned();
        }
    }
    Ok(Some((event.unwrap_or_default(), data)))
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
