use crate::redact::redact_text;
use sim_kernel::{Error, Result};
use std::{
    io::{BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

mod body;

use body::{read_chunked_transfer_body, read_content_length_body, read_to_end_limited};

type BodyChunkCallback<'a> = &'a mut dyn FnMut(&[u8]) -> Result<()>;
type OptionalBodyChunkCallback<'a> = Option<BodyChunkCallback<'a>>;

#[cfg(feature = "tls")]
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, StreamOwned,
    pki_types::{CertificateDer, ServerName},
};
#[cfg(feature = "tls")]
use std::sync::Arc;

pub(crate) struct HttpRunnerRequest<'a> {
    pub(crate) runner_label: &'a str,
    pub(crate) endpoint: &'a str,
    pub(crate) path: &'a str,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Duration,
    pub(crate) body: Vec<u8>,
    pub(crate) max_response_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpRunnerResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedUrl {
    scheme: UrlScheme,
    host: String,
    port: u16,
    path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UrlScheme {
    Http,
    #[cfg(feature = "tls")]
    Https,
}

fn parse_url(runner_label: &str, url: &str) -> Result<ParsedUrl> {
    // Classify and gate the scheme exactly, in the same
    // order (scheme rejection precedes host validation), then delegate the
    // structural parse to the shared net-core primitive and map its path
    // convention and error text to this client's contract.
    let (scheme_text, _) = url
        .split_once("://")
        .ok_or_else(|| Error::Eval(format!("invalid url {url}")))?;
    let scheme = match scheme_text {
        "http" => UrlScheme::Http,
        "https" => {
            #[cfg(not(feature = "tls"))]
            {
                return Err(Error::Eval(format!(
                    "{runner_label} https endpoints require the sim feature agent-runner-http-tls"
                )));
            }
            #[cfg(feature = "tls")]
            {
                UrlScheme::Https
            }
        }
        other => {
            return Err(Error::Eval(format!(
                "{runner_label} unsupported url scheme {other}"
            )));
        }
    };
    let parts = sim_lib_net_core::parse_url(url).map_err(|err| map_url_error(url, err))?;
    // net-core defaults an absent path to "/"; this client uses an
    // empty path so the request line is `POST <request.path> ...`. Restore the
    // empty default only when the URL carries no path component at all (i.e. no
    // `/` after `scheme://`), preserving byte-identical request lines.
    let path = if url_has_path_component(url) {
        parts.path
    } else {
        String::new()
    };
    Ok(ParsedUrl {
        scheme,
        host: parts.host,
        port: parts.port,
        path,
    })
}

/// Whether the URL has a path component (a `/` after `scheme://host[:port]`).
fn url_has_path_component(url: &str) -> bool {
    url.split_once("://")
        .map(|(_, rest)| rest.contains('/'))
        .unwrap_or(false)
}

/// Map a net-core URL error to this client's exact error text. The scheme
/// has already been classified and gated by [`parse_url`]; the only remaining
/// failures are an invalid port or an empty host.
fn map_url_error(url: &str, error: sim_lib_net_core::NetError) -> Error {
    use sim_lib_net_core::NetError;
    match error {
        NetError::InvalidPort(_) => Error::Eval(format!("invalid port in url {url}")),
        // For http/https (the only schemes reaching here), the remaining
        // MalformedUrl / UnsupportedScheme cases are the empty-host condition.
        _ => Error::Eval(format!("url missing host in {url}")),
    }
}

fn read_response(
    runner_label: &str,
    stream: &mut dyn Read,
    max_response_bytes: usize,
    secret: Option<&str>,
    on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<HttpRunnerResponse> {
    let mut reader = BufReader::new(stream);
    // 64 KiB HTTP head cap (caller policy); the read-until-CRLFCRLF framing is
    // the shared net-core primitive.
    let head = match sim_lib_net_core::read_head_until_double_crlf(&mut reader, 64 * 1024) {
        Ok(sim_lib_net_core::HeadOutcome::Head(head)) => head,
        Ok(sim_lib_net_core::HeadOutcome::TooLarge) => {
            return Err(Error::HostError(
                "http headers exceed size limit".to_owned(),
            ));
        }
        Ok(sim_lib_net_core::HeadOutcome::Eof | sim_lib_net_core::HeadOutcome::Truncated(_)) => {
            return Err(host_error(
                runner_label,
                std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
                secret,
            ));
        }
        Err(err) => return Err(host_error(runner_label, err, secret)),
    };
    let head_text = std::str::from_utf8(&head)
        .map_err(|_| Error::HostError("http headers are not valid utf-8".to_owned()))?;
    // Parse the status line + headers with the shared net-core primitive, then
    // classify the body framing. All socket I/O below stays local.
    let parsed = sim_lib_net_core::parse_http_head(head_text).map_err(map_head_error)?;
    let status = parsed.status;
    let mode = sim_lib_net_core::body_mode(&parsed).map_err(map_head_error)?;
    match mode {
        sim_lib_net_core::HttpBodyMode::Chunked => {
            let body = read_chunked_transfer_body(
                runner_label,
                &mut reader,
                max_response_bytes,
                secret,
                on_body_chunk,
            )?;
            Ok(HttpRunnerResponse { status, body })
        }
        // `body_mode` maps both a missing and a literal `Content-Length: 0` to
        // `UntilEof`, matching the read-to-end-on-zero-length behavior.
        sim_lib_net_core::HttpBodyMode::UntilEof | sim_lib_net_core::HttpBodyMode::Empty => {
            let body = read_to_end_limited(
                runner_label,
                &mut reader,
                max_response_bytes,
                secret,
                on_body_chunk,
            )?;
            Ok(HttpRunnerResponse { status, body })
        }
        sim_lib_net_core::HttpBodyMode::ContentLength(content_length) => {
            if content_length > max_response_bytes {
                return Err(Error::Eval(format!(
                    "{runner_label} response exceeded max output bytes {max_response_bytes}"
                )));
            }
            let body = read_content_length_body(
                runner_label,
                &mut reader,
                content_length,
                secret,
                on_body_chunk,
            )?;
            Ok(HttpRunnerResponse { status, body })
        }
    }
}

/// Map a net-core head-parse error to this client's exact error text.
fn map_head_error(error: sim_lib_net_core::NetError) -> Error {
    use sim_lib_net_core::NetError;
    let message = match error {
        NetError::InvalidHead(detail) => match detail.as_str() {
            "missing status line" => "http response missing status line",
            "missing version" => "http response missing version",
            "missing status" => "http response missing status",
            "invalid status code" => "invalid http response status",
            "invalid header line" => "invalid http header line",
            "invalid content-length" => "invalid content-length",
            _ => "invalid http head",
        },
        _ => "invalid http head",
    };
    Error::HostError(message.to_owned())
}

fn host_error(runner_label: &str, error: std::io::Error, secret: Option<&str>) -> Error {
    Error::HostError(redact(
        &format!("{runner_label} io {:?}: {}", error.kind(), error),
        secret,
    ))
}

fn redact(text: &str, secret: Option<&str>) -> String {
    match secret {
        Some(secret) => redact_text(text, &[secret]),
        None => text.to_owned(),
    }
}

trait ReadWrite: Read + Write {}

impl<T: Read + Write> ReadWrite for T {}

pub(crate) fn post_json(
    request: HttpRunnerRequest<'_>,
    secret: Option<&str>,
) -> Result<HttpRunnerResponse> {
    #[cfg(feature = "tls")]
    {
        post_json_with_tls_roots(request, secret, None)
    }
    #[cfg(not(feature = "tls"))]
    {
        post_json_with_tls_roots(request, secret)
    }
}

pub(crate) fn post_json_stream(
    request: HttpRunnerRequest<'_>,
    secret: Option<&str>,
    on_body_chunk: BodyChunkCallback<'_>,
) -> Result<HttpRunnerResponse> {
    #[cfg(feature = "tls")]
    {
        post_json_with_tls_roots_stream(request, secret, None, on_body_chunk)
    }
    #[cfg(not(feature = "tls"))]
    {
        post_json_with_tls_roots_stream(request, secret, on_body_chunk)
    }
}

fn post_json_with_tls_roots(
    request: HttpRunnerRequest<'_>,
    secret: Option<&str>,
    #[cfg(feature = "tls")] tls_roots: Option<Vec<CertificateDer<'static>>>,
) -> Result<HttpRunnerResponse> {
    #[cfg(feature = "tls")]
    {
        post_json_with_tls_roots_impl(request, secret, tls_roots, None)
    }
    #[cfg(not(feature = "tls"))]
    {
        post_json_with_tls_roots_impl(request, secret, None)
    }
}

fn post_json_with_tls_roots_stream(
    request: HttpRunnerRequest<'_>,
    secret: Option<&str>,
    #[cfg(feature = "tls")] tls_roots: Option<Vec<CertificateDer<'static>>>,
    on_body_chunk: BodyChunkCallback<'_>,
) -> Result<HttpRunnerResponse> {
    #[cfg(feature = "tls")]
    {
        post_json_with_tls_roots_impl(request, secret, tls_roots, Some(on_body_chunk))
    }
    #[cfg(not(feature = "tls"))]
    {
        post_json_with_tls_roots_impl(request, secret, Some(on_body_chunk))
    }
}

fn post_json_with_tls_roots_impl(
    request: HttpRunnerRequest<'_>,
    secret: Option<&str>,
    #[cfg(feature = "tls")] tls_roots: Option<Vec<CertificateDer<'static>>>,
    on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<HttpRunnerResponse> {
    let url = parse_url(request.runner_label, request.endpoint)?;
    let stream = TcpStream::connect((url.host.as_str(), url.port))
        .map_err(|err| host_error(request.runner_label, err, secret))?;
    stream
        .set_read_timeout(Some(request.timeout))
        .map_err(|err| host_error(request.runner_label, err, secret))?;
    stream
        .set_write_timeout(Some(request.timeout))
        .map_err(|err| host_error(request.runner_label, err, secret))?;
    #[cfg(feature = "tls")]
    let mut stream = connect_stream(request.runner_label, &url, stream, secret, tls_roots)?;
    #[cfg(not(feature = "tls"))]
    let mut stream = connect_stream(request.runner_label, &url, stream, secret)?;
    let mut head = format!(
        "POST {}{} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        url.path,
        request.path,
        url.host,
        request.body.len(),
    );
    for (name, value) in &request.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    write!(stream, "{head}").map_err(|err| host_error(request.runner_label, err, secret))?;
    stream
        .write_all(&request.body)
        .map_err(|err| host_error(request.runner_label, err, secret))?;
    stream
        .flush()
        .map_err(|err| host_error(request.runner_label, err, secret))?;
    let response = read_response(
        request.runner_label,
        &mut stream,
        request.max_response_bytes,
        secret,
        on_body_chunk,
    )?;
    if !(200..300).contains(&response.status) {
        let body = String::from_utf8_lossy(&response.body);
        return Err(Error::Eval(format!(
            "{} http {}: {}",
            request.runner_label,
            response.status,
            redact(&body, secret)
        )));
    }
    Ok(response)
}

fn connect_stream(
    _runner_label: &str,
    url: &ParsedUrl,
    stream: TcpStream,
    _secret: Option<&str>,
    #[cfg(feature = "tls")] tls_roots: Option<Vec<CertificateDer<'static>>>,
) -> Result<Box<dyn ReadWrite>> {
    match url.scheme {
        UrlScheme::Http => Ok(Box::new(stream)),
        #[cfg(feature = "tls")]
        UrlScheme::Https => {
            let config = tls_client_config(_runner_label, _secret, tls_roots)?;
            let server_name = ServerName::try_from(url.host.clone()).map_err(|_| {
                Error::Eval(format!(
                    "{} invalid tls server name {}",
                    _runner_label, url.host
                ))
            })?;
            let connection = ClientConnection::new(config, server_name)
                .map_err(|err| Error::HostError(redact(&err.to_string(), _secret)))?;
            Ok(Box::new(StreamOwned::new(connection, stream)))
        }
    }
}

#[cfg(feature = "tls")]
fn tls_client_config(
    runner_label: &str,
    secret: Option<&str>,
    extra_roots: Option<Vec<CertificateDer<'static>>>,
) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    for certificate in cert_result.certs {
        roots
            .add(certificate)
            .map_err(|err| Error::HostError(redact(&err.to_string(), secret)))?;
    }
    if let Some(extra_roots) = extra_roots {
        for certificate in extra_roots {
            roots
                .add(certificate)
                .map_err(|err| Error::HostError(redact(&err.to_string(), secret)))?;
        }
    }
    if roots.is_empty() {
        return Err(Error::HostError(format!(
            "{runner_label} no tls root certificates available"
        )));
    }
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(all(test, feature = "tls"))]
mod tests;
