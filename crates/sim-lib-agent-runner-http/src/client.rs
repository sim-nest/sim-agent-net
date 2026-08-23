use crate::redact::redact_text;
use sim_kernel::{Error, Result};
use sim_lib_provider::Secret;
use std::{
    io::{Read, Write},
    time::Duration,
};

type BodyChunkCallback<'a> = &'a mut dyn FnMut(&[u8]) -> Result<()>;
type OptionalBodyChunkCallback<'a> = Option<BodyChunkCallback<'a>>;

#[cfg(feature = "tls")]
use rustls::pki_types::CertificateDer;

#[derive(Clone, Debug)]
pub(crate) struct HttpRunnerRequest {
    pub(crate) runner_label: &'static str,
    pub(crate) endpoint: String,
    pub(crate) path: &'static str,
    pub(crate) headers: Vec<(String, Secret)>,
    pub(crate) timeout: Duration,
    pub(crate) body: Vec<u8>,
    pub(crate) max_response_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpRunnerResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

fn redact(text: &str, secret: Option<&str>) -> String {
    match secret {
        Some(secret) => redact_text(text, &[secret]),
        None => text.to_owned(),
    }
}

pub(crate) fn post_json(
    request: HttpRunnerRequest,
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
    request: HttpRunnerRequest,
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
    request: HttpRunnerRequest,
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
    request: HttpRunnerRequest,
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
    request: HttpRunnerRequest,
    secret: Option<&str>,
    #[cfg(feature = "tls")] tls_roots: Option<Vec<CertificateDer<'static>>>,
    mut on_body_chunk: OptionalBodyChunkCallback<'_>,
) -> Result<HttpRunnerResponse> {
    use sim_lib_net_http::{
        Cancellation, Client, Header, Method, Policy, Request, RequestBody, Url,
    };
    let endpoint = format!("{}{}", request.endpoint.trim_end_matches('/'), request.path);
    #[cfg(not(feature = "tls"))]
    if endpoint.starts_with("https://") {
        return Err(Error::Eval(format!(
            "{} https endpoints require the sim feature agent-runner-http-tls",
            request.runner_label
        )));
    }
    let url = Url::parse(endpoint)
        .map_err(|error| Error::Eval(format!("{} {error}", request.runner_label)))?;
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            Header::sensitive(name.clone(), value.expose().to_owned())
                .map_err(|error| Error::Eval(format!("{} {error}", request.runner_label)))
        })
        .collect::<Result<Vec<_>>>()?;
    #[cfg(feature = "tls")]
    let tls_root_certificates = tls_roots
        .unwrap_or_default()
        .into_iter()
        .map(|root| root.as_ref().to_vec())
        .collect();
    #[cfg(not(feature = "tls"))]
    let tls_root_certificates = Vec::new();
    let policy = Policy {
        connect_timeout: request.timeout,
        read_timeout: request.timeout,
        write_timeout: request.timeout,
        total_timeout: request.timeout,
        max_request_bytes: request.body.len(),
        max_response_bytes: request.max_response_bytes,
        max_decompressed_bytes: request.max_response_bytes,
        tls_root_certificates,
        ..Policy::default()
    };
    let client = Client::new(
        CapsuleConnector {
            runner_label: request.runner_label,
            secret,
        },
        policy,
    );
    let mut shared_request = Request {
        method: Method::post(),
        url,
        headers,
        body: RequestBody::Bytes(&request.body),
        deadline: None,
        cancellation: Cancellation::default(),
    };
    let response = client
        .execute_stream(&mut shared_request, |chunk| match on_body_chunk.as_mut() {
            Some(callback) => callback(chunk).map_err(|error| {
                sim_lib_net_http::Error::Protocol(redact(&error.to_string(), secret))
            }),
            None => Ok(()),
        })
        .map_err(|error| {
            Error::HostError(redact(
                &format!("{} http: {error}", request.runner_label),
                secret,
            ))
        })?;
    let response = HttpRunnerResponse {
        status: response.status,
        body: response.into_body(),
    };
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

struct CapsuleConnector<'a> {
    runner_label: &'static str,
    secret: Option<&'a str>,
}

impl sim_lib_net_http::Connector for CapsuleConnector<'_> {
    fn connect(
        &self,
        url: &sim_lib_net_http::Url,
        _policy: &sim_lib_net_http::Policy,
    ) -> sim_lib_net_http::Result<Box<dyn sim_lib_net_http::Connection>> {
        let services = match sim_transport_ports::services() {
            Ok(services) => services,
            #[cfg(test)]
            Err(_) => {
                return sim_lib_net_http::Connector::connect(
                    &sim_lib_net_http::TcpConnector,
                    url,
                    _policy,
                );
            }
            #[cfg(not(test))]
            Err(error) => {
                return Err(sim_lib_net_http::Error::Connect(redact(
                    &format!("{} transport: {error}", self.runner_label),
                    self.secret,
                )));
            }
        };
        let addresses = services
            .dns
            .resolve(url.host(), url.port())
            .map_err(|error| {
                sim_lib_net_http::Error::Dns(redact(
                    &format!("{} transport: {error}", self.runner_label),
                    self.secret,
                ))
            })?;
        let address = addresses.first().ok_or_else(|| {
            sim_lib_net_http::Error::Dns(format!("{} DNS returned no addresses", self.runner_label))
        })?;
        let stream = services.sockets.connect_tcp(address).map_err(|error| {
            sim_lib_net_http::Error::Connect(redact(
                &format!("{} transport: {error}", self.runner_label),
                self.secret,
            ))
        })?;
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
impl sim_lib_net_http::Connection for CapsuleConnection {
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

#[cfg(all(test, feature = "tls"))]
mod tests;
