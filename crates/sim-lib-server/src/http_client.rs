use sim_kernel::{Error, Result};
use sim_lib_net_http::{
    Cancellation, Client, Connection, Connector, Method, Policy, Request, RequestBody, Url,
};
use std::{
    io::{Read, Write},
    time::Duration,
};

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

/// Performs a bounded blocking HTTP GET through the bound capsule transport.
pub fn http_get(url: &str, timeout: Duration, max_body_bytes: usize) -> Result<HttpGetResponse> {
    let policy = Policy {
        connect_timeout: timeout,
        read_timeout: timeout,
        write_timeout: timeout,
        total_timeout: timeout,
        max_response_bytes: max_body_bytes,
        max_decompressed_bytes: max_body_bytes,
        ..Policy::default()
    };
    let response = Client::new(CapsuleConnector, policy)
        .execute(Request {
            method: Method::get(),
            url: Url::parse(url).map_err(map_http_error)?,
            headers: Vec::new(),
            body: RequestBody::Empty,
            deadline: None,
            cancellation: Cancellation::default(),
        })
        .map_err(map_http_error)?;
    Ok(HttpGetResponse {
        status: response.status,
        url: url.to_owned(),
        body: response.into_body(),
    })
}

struct CapsuleConnector;
impl Connector for CapsuleConnector {
    fn connect(
        &self,
        url: &Url,
        _policy: &Policy,
    ) -> sim_lib_net_http::Result<Box<dyn Connection>> {
        let services = sim_transport_ports::services()
            .map_err(|error| sim_lib_net_http::Error::Connect(error.to_string()))?;
        let addresses = services
            .dns
            .resolve(url.host(), url.port())
            .map_err(|error| sim_lib_net_http::Error::Dns(error.to_string()))?;
        let address = addresses.first().ok_or_else(|| {
            sim_lib_net_http::Error::Dns("capsule DNS returned no addresses".to_owned())
        })?;
        let stream = services
            .sockets
            .connect_tcp(address)
            .map_err(|error| sim_lib_net_http::Error::Connect(error.to_string()))?;
        Ok(Box::new(CapsuleConnection(stream)))
    }
}

struct CapsuleConnection(Box<dyn sim_transport_ports::Stream>);
impl Read for CapsuleConnection {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}
impl Write for CapsuleConnection {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl Connection for CapsuleConnection {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.0
            .set_read_timeout(timeout)
            .map_err(std::io::Error::other)
    }
    fn set_write_timeout(&self, _timeout: Option<Duration>) -> std::io::Result<()> {
        Ok(())
    }
}
impl Drop for CapsuleConnection {
    fn drop(&mut self) {
        let _ = self.0.shutdown(sim_transport_ports::Half::Both);
    }
}

fn map_http_error(error: sim_lib_net_http::Error) -> Error {
    match error {
        sim_lib_net_http::Error::InvalidUrl
        | sim_lib_net_http::Error::UserInfoForbidden
        | sim_lib_net_http::Error::UnsupportedScheme
        | sim_lib_net_http::Error::RequestTooLarge { .. }
        | sim_lib_net_http::Error::ResponseTooLarge { .. } => Error::Eval(error.to_string()),
        _ => Error::HostError(error.to_string()),
    }
}
