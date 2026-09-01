/// Secret reference understood only by the injected resolver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrincipalRef(pub String);

/// S1 boundary: implementations return header bytes without making them durable.
pub trait SecretResolver: Send + Sync {
    fn principal_headers(
        &self,
        principal: &PrincipalRef,
    ) -> Result<Vec<(String, String)>, SearchHttpError>;
}

/// Shared HTTP membrane seam; tests and host capsules inject an implementation.
pub trait SearchHttpClient: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, SearchHttpError>;
}

/// Request admitted by the transport policy.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub response_limit: usize,
}
/// Raw response captured before provider decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Production adapter over the shared `sim-lib-net-http` membrane.
pub struct MembraneHttpClient<C> {
    connector: C,
    policy: HttpPolicy,
}
impl<C> MembraneHttpClient<C> {
    pub fn new(connector: C, policy: HttpPolicy) -> Self {
        Self { connector, policy }
    }
}
impl<C: Connector + Clone> SearchHttpClient for MembraneHttpClient<C> {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, SearchHttpError> {
        let mut policy = self.policy.clone();
        policy.max_response_bytes = policy.max_response_bytes.min(request.response_limit);
        policy.connect_timeout = policy.connect_timeout.min(request.timeout);
        policy.read_timeout = policy.read_timeout.min(request.timeout);
        let client = Client::new(self.connector.clone(), policy);
        let headers = request
            .headers
            .iter()
            .map(|(n, v)| Header::new(n, v).map_err(wire))
            .collect::<Result<Vec<_>, _>>()?;
        let response = client
            .execute(Request {
                method: Method::new("POST").map_err(wire)?,
                url: Url::parse(&request.endpoint).map_err(wire)?,
                headers,
                body: RequestBody::Bytes(&request.body),
                deadline: None,
                cancellation: Default::default(),
            })
            .map_err(wire)?;
        Ok(HttpResponse {
            status: response.status,
            headers: response
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name().into(),
                        if h.is_sensitive() {
                            "[REDACTED]".into()
                        } else {
                            h.value().into()
                        },
                    )
                })
                .collect(),
            body: response.into_body(),
        })
    }
}
fn wire(error: sim_lib_net_http::Error) -> SearchHttpError {
    SearchHttpError::Transport(error.to_string())
}
