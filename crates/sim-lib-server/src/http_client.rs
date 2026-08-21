use crate::http::{HttpRequest, parse_url, read_response, write_request};
use crate::transport::port_io::PortTcpStream as TcpStream;
use sim_kernel::{Error, Result};
use std::time::Duration;

/// Outcome of an HTTP GET request issued by the server library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpGetResponse {
    /// HTTP status code returned by the remote server.
    pub status: u16,
    /// URL that was requested.
    pub url: String,
    /// Raw response body bytes.
    pub body: Vec<u8>,
}

/// Performs a blocking HTTP GET against `url`, returning the response.
///
/// Applies `timeout` to both the read and write halves of the connection and
/// fails with an evaluation error when the body exceeds `max_body_bytes`.
pub fn http_get(url: &str, timeout: Duration, max_body_bytes: usize) -> Result<HttpGetResponse> {
    let parsed = parse_url(url, "http", "/")?;
    let mut stream =
        TcpStream::connect((parsed.host.as_str(), parsed.port)).map_err(host_io_error)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(host_io_error)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(host_io_error)?;
    write_request(
        &mut stream,
        &HttpRequest {
            method: "GET".to_owned(),
            path: parsed.path.clone(),
            headers: vec![("Host".to_owned(), parsed.host.clone())],
            body: Vec::new(),
        },
    )?;
    let response = read_response(&mut stream)?;
    if response.body.len() > max_body_bytes {
        return Err(Error::Eval(format!(
            "http response exceeded {max_body_bytes} bytes"
        )));
    }
    Ok(HttpGetResponse {
        status: response.status,
        url: url.to_owned(),
        body: response.body,
    })
}

fn host_io_error(error: std::io::Error) -> Error {
    Error::HostError(error.to_string())
}
