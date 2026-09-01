use super::{ProbeHttpRequest, ProbeHttpResponse, ProbeTransport};
use sim_kernel::{Error, Result};
use sim_lib_net_core::HttpHead;
use sim_lib_net_http::{
    Cancellation, Client, Header, Method, Policy, Request, RequestBody, TcpConnector, Url,
};

/// HTTP/HTTPS probe transport over the constellation's shared policy boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpProbeTransport;

impl ProbeTransport for HttpProbeTransport {
    fn get(&self, request: ProbeHttpRequest<'_>) -> Result<ProbeHttpResponse> {
        let endpoint = format!(
            "{}/{}",
            request.endpoint.trim_end_matches('/'),
            request.path.trim_start_matches('/')
        );
        let headers = std::iter::once(Header::new("Accept", "application/json"))
            .chain(request.headers.into_iter().map(|(name, value)| {
                if is_sensitive(&name) {
                    Header::sensitive(name, value)
                } else {
                    Header::new(name, value)
                }
            }))
            .collect::<sim_lib_net_http::Result<Vec<_>>>()
            .map_err(map_http_error)?;
        let policy = Policy {
            connect_timeout: request.timeout,
            read_timeout: request.timeout,
            write_timeout: request.timeout,
            total_timeout: request.timeout,
            max_response_bytes: request.max_response_bytes,
            max_decompressed_bytes: request.max_response_bytes,
            ..Policy::default()
        };
        let response = Client::new(TcpConnector, policy)
            .execute(Request {
                method: Method::get(),
                url: Url::parse(endpoint).map_err(map_http_error)?,
                headers,
                body: RequestBody::Empty,
                deadline: None,
                cancellation: Cancellation::default(),
            })
            .map_err(map_http_error)?;
        let head = HttpHead {
            status: response.status,
            reason: response.reason.clone(),
            headers: response
                .headers
                .iter()
                .map(|header| (header.name().to_owned(), header.value().to_owned()))
                .collect(),
        };
        Ok(ProbeHttpResponse {
            head,
            body: response.into_body(),
        })
    }
}

fn is_sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie"
    )
}

fn map_http_error(error: sim_lib_net_http::Error) -> Error {
    match error {
        sim_lib_net_http::Error::InvalidUrl
        | sim_lib_net_http::Error::UserInfoForbidden
        | sim_lib_net_http::Error::UnsupportedScheme
        | sim_lib_net_http::Error::InvalidHeaderName
        | sim_lib_net_http::Error::InvalidHeaderValue
        | sim_lib_net_http::Error::AmbiguousHeader
        | sim_lib_net_http::Error::UnsupportedTransferFraming
        | sim_lib_net_http::Error::RequestTooLarge { .. }
        | sim_lib_net_http::Error::ResponseTooLarge { .. } => {
            Error::Eval(format!("provider/probe {error}"))
        }
        _ => Error::HostError(format!("provider/probe {error}")),
    }
}
