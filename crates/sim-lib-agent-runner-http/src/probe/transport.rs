use super::{ProbeHttpRequest, ProbeHttpResponse, ProbeTransport};
#[cfg(feature = "tls")]
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use sim_kernel::{Error, Result};
use sim_lib_net_core::{
    HeadOutcome, HttpBodyMode, UrlParts, body_mode, parse_http_head, read_head_until_double_crlf,
};
#[cfg(feature = "tls")]
use std::sync::Arc;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
};

/// Plain HTTP/HTTPS probe transport used by the agent-facing `provider/probe` function.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpProbeTransport;

impl ProbeTransport for HttpProbeTransport {
    fn get(&self, request: ProbeHttpRequest<'_>) -> Result<ProbeHttpResponse> {
        let stream = TcpStream::connect((
            request.endpoint_parts.host.as_str(),
            request.endpoint_parts.port,
        ))
        .map_err(|error| {
            Error::HostError(format!("provider/probe io {:?}: {}", error.kind(), error))
        })?;
        stream
            .set_read_timeout(Some(request.timeout))
            .map_err(map_io_error)?;
        stream
            .set_write_timeout(Some(request.timeout))
            .map_err(map_io_error)?;
        let mut stream = connect_probe_stream(&request.endpoint_parts, stream)?;

        let target = join_paths(&request.endpoint_parts.path, request.path);
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            host_header(&request.endpoint_parts)
        )
        .map_err(map_io_error)?;
        for (name, value) in &request.headers {
            write!(stream, "{name}: {value}\r\n").map_err(map_io_error)?;
        }
        write!(stream, "\r\n").map_err(map_io_error)?;
        stream.flush().map_err(map_io_error)?;
        read_probe_response(&mut stream, request.max_response_bytes)
    }
}

trait ProbeReadWrite: Read + Write {}

impl<T: Read + Write> ProbeReadWrite for T {}

#[cfg(feature = "tls")]
fn connect_probe_stream(parts: &UrlParts, stream: TcpStream) -> Result<Box<dyn ProbeReadWrite>> {
    match parts.scheme.as_str() {
        "http" => Ok(Box::new(stream)),
        "https" => {
            let server_name = ServerName::try_from(parts.host.clone()).map_err(|_| {
                Error::Eval(format!(
                    "provider/probe invalid tls server name {}",
                    parts.host
                ))
            })?;
            let connection = ClientConnection::new(tls_client_config()?, server_name)
                .map_err(|error| Error::HostError(error.to_string()))?;
            Ok(Box::new(StreamOwned::new(connection, stream)))
        }
        other => Err(Error::Eval(format!(
            "provider/probe unsupported url scheme {other}"
        ))),
    }
}

#[cfg(not(feature = "tls"))]
fn connect_probe_stream(parts: &UrlParts, stream: TcpStream) -> Result<Box<dyn ProbeReadWrite>> {
    match parts.scheme.as_str() {
        "http" => Ok(Box::new(stream)),
        "https" => Err(Error::Eval(
            "provider/probe https endpoints require the tls feature".to_owned(),
        )),
        other => Err(Error::Eval(format!(
            "provider/probe unsupported url scheme {other}"
        ))),
    }
}

#[cfg(feature = "tls")]
fn tls_client_config() -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    for certificate in cert_result.certs {
        roots
            .add(certificate)
            .map_err(|error| Error::HostError(error.to_string()))?;
    }
    if roots.is_empty() {
        return Err(Error::HostError(
            "provider/probe no tls root certificates available".to_owned(),
        ));
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

fn read_probe_response(
    stream: &mut dyn Read,
    max_response_bytes: usize,
) -> Result<ProbeHttpResponse> {
    let mut reader = BufReader::new(stream);
    let head = match read_head_until_double_crlf(&mut reader, 64 * 1024) {
        Ok(HeadOutcome::Head(head)) => head,
        Ok(HeadOutcome::TooLarge) => {
            return Err(Error::HostError(
                "provider/probe http headers exceed size limit".to_owned(),
            ));
        }
        Ok(HeadOutcome::Eof | HeadOutcome::Truncated(_)) => {
            return Err(Error::HostError(
                "provider/probe http response ended before headers".to_owned(),
            ));
        }
        Err(error) => return Err(Error::HostError(error.to_string())),
    };
    let head_text = std::str::from_utf8(&head)
        .map_err(|_| Error::HostError("provider/probe http headers are not utf-8".to_owned()))?;
    let parsed = parse_http_head(head_text).map_err(|error| Error::HostError(error.to_string()))?;
    let mode = body_mode(&parsed).map_err(|error| Error::HostError(error.to_string()))?;
    let body = match mode {
        HttpBodyMode::ContentLength(length) => {
            read_content_length(&mut reader, length, max_response_bytes)?
        }
        HttpBodyMode::Chunked => read_chunked(&mut reader, max_response_bytes)?,
        HttpBodyMode::UntilEof | HttpBodyMode::Empty => {
            read_to_end(&mut reader, max_response_bytes)?
        }
    };
    Ok(ProbeHttpResponse { head: parsed, body })
}

fn read_content_length(
    reader: &mut dyn Read,
    length: usize,
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    if length > max_response_bytes {
        return Err(Error::Eval(format!(
            "provider/probe response exceeded max output bytes {max_response_bytes}"
        )));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(map_io_error)?;
    Ok(body)
}

fn read_to_end(reader: &mut dyn Read, max_response_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > max_response_bytes {
            return Err(Error::Eval(format!(
                "provider/probe response exceeded max output bytes {max_response_bytes}"
            )));
        }
    }
    Ok(body)
}

fn read_chunked(reader: &mut dyn BufRead, max_response_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line).map_err(map_io_error)?;
        let raw_size = size_line.trim_end_matches("\r\n");
        let size_text = raw_size.split(';').next().unwrap_or(raw_size);
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| Error::HostError("provider/probe invalid chunk size".to_owned()))?;
        if size == 0 {
            drain_chunk_trailers(reader)?;
            return Ok(body);
        }
        if body.len().saturating_add(size) > max_response_bytes {
            return Err(Error::Eval(format!(
                "provider/probe response exceeded max output bytes {max_response_bytes}"
            )));
        }
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk).map_err(map_io_error)?;
        body.extend_from_slice(&chunk);
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).map_err(map_io_error)?;
        if crlf != *b"\r\n" {
            return Err(Error::HostError(
                "provider/probe invalid chunk terminator".to_owned(),
            ));
        }
    }
}

fn drain_chunk_trailers(reader: &mut dyn BufRead) -> Result<()> {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(map_io_error)?;
        if line == "\r\n" || line.is_empty() {
            return Ok(());
        }
    }
}

fn join_paths(base: &str, suffix: &str) -> String {
    let base = base.trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');
    if base.is_empty() {
        format!("/{suffix}")
    } else {
        format!("{base}/{suffix}")
    }
}

fn host_header(parts: &UrlParts) -> String {
    let default_port = (parts.scheme == "http" && parts.port == 80)
        || (parts.scheme == "https" && parts.port == 443);
    if default_port {
        parts.host.clone()
    } else {
        format!("{}:{}", parts.host, parts.port)
    }
}

fn map_io_error(error: std::io::Error) -> Error {
    Error::HostError(format!("provider/probe io {:?}: {}", error.kind(), error))
}
