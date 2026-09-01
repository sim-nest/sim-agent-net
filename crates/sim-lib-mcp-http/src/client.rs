/// Metadata projected by the client onto one final-protocol request.
#[derive(Clone, Debug)]
pub struct ClientRequest {
    /// Request endpoint URL.
    pub url: net_http::Url,
    /// Optional sensitive bearer credential.
    pub authorization: Option<String>,
    /// Whether a streaming response is preferred.
    pub streaming: bool,
    /// Cooperative request cancellation.
    pub cancellation: net_http::Cancellation,
    /// Optional absolute deadline.
    pub deadline: Option<Instant>,
}

/// Classified final-protocol HTTP outcome.
#[derive(Debug)]
pub enum ClientOutcome {
    /// Accepted notification with no body.
    Accepted,
    /// One JSON-RPC response or error.
    Json(McpEnvelope),
    /// Ordered complete SSE message frames.
    Stream(Vec<McpEnvelope>),
    /// Discovery probe classification without treating non-success as transport failure.
    Discovery {
        /// HTTP status returned by the probed endpoint.
        status: u16,
        /// Whether this is a final-protocol MCP endpoint.
        final_protocol: bool,
    },
}

/// Client wire adapter over the shared bounded HTTP client.
pub struct McpHttpClient<C> {
    client: net_http::Client<C>,
    codec: Mutex<Cx>,
}
impl<C: net_http::Connector> McpHttpClient<C> {
    /// Creates an adapter. `codec_cx` must have `codec:mcp` loaded.
    pub fn new(client: net_http::Client<C>, codec_cx: Cx) -> Self {
        Self {
            client,
            codec: Mutex::new(codec_cx),
        }
    }
    /// Sends one request or notification with exact negotiation and bounded response parsing.
    pub fn send(
        &self,
        metadata: ClientRequest,
        envelope: &McpEnvelope,
    ) -> std::result::Result<ClientOutcome, net_http::Error> {
        let bytes =
            encode(&self.codec, envelope).map_err(|e| net_http::Error::Protocol(e.to_string()))?;
        let method = match envelope {
            McpEnvelope::Request(r) => r.method.as_str(),
            McpEnvelope::Notification(n) => n.method.as_str(),
            _ => {
                return Err(net_http::Error::Protocol(
                    "client body must be request or notification".into(),
                ));
            }
        };
        let mut headers = vec![
            net_http::Header::new("Content-Type", JSON)?,
            net_http::Header::new(
                "Accept",
                if metadata.streaming {
                    format!("{JSON}, {SSE}")
                } else {
                    JSON.into()
                },
            )?,
            net_http::Header::new("MCP-Protocol-Version", PROTOCOL)?,
            net_http::Header::new("MCP-Method", method)?,
        ];
        if let Some(name) = envelope_params(envelope).and_then(|p| expr_field(p, "name")) {
            headers.push(net_http::Header::new("MCP-Name", name)?);
        }
        if let Some(secret) = metadata.authorization {
            headers.push(net_http::Header::sensitive("Authorization", secret)?);
        }
        let mut request = net_http::Request {
            method: net_http::Method::post(),
            url: metadata.url,
            headers,
            body: net_http::RequestBody::Bytes(&bytes),
            deadline: metadata.deadline,
            cancellation: metadata.cancellation,
        };
        let mut streamed = Vec::new();
        let response = self.client.execute_stream(&mut request, |chunk| {
            streamed.extend_from_slice(chunk);
            Ok(())
        })?;
        classify_response(
            response.status,
            &response.headers,
            &streamed,
            &self.codec,
            false,
        )
    }
    /// Probes an endpoint and returns classification even for an HTTP non-success.
    pub fn classify_discovery(
        &self,
        status: u16,
        headers: &[net_http::Header],
        body: &[u8],
    ) -> std::result::Result<ClientOutcome, net_http::Error> {
        classify_response(status, headers, body, &self.codec, true)
    }
}
fn envelope_params(envelope: &McpEnvelope) -> Option<&Expr> {
    match envelope {
        McpEnvelope::Request(r) => Some(&r.params),
        McpEnvelope::Notification(n) => Some(&n.params),
        _ => None,
    }
}
fn classify_response(
    status: u16,
    headers: &[net_http::Header],
    body: &[u8],
    codec: &Mutex<Cx>,
    probe: bool,
) -> std::result::Result<ClientOutcome, net_http::Error> {
    let content = headers
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case("content-type"))
        .map(net_http::Header::value);
    if probe {
        return Ok(ClientOutcome::Discovery {
            status,
            final_protocol: matches!(status, 200 | 202)
                && content.is_some_and(|v| media_type(v) == JSON || media_type(v) == SSE),
        });
    }
    if status == 202 && body.is_empty() {
        return Ok(ClientOutcome::Accepted);
    }
    if status != 200 {
        return Err(net_http::Error::Protocol(format!(
            "MCP HTTP status {status}"
        )));
    }
    match content.map(media_type) {
        Some(JSON) => decode(codec, body.to_vec())
            .map(ClientOutcome::Json)
            .map_err(|e| net_http::Error::Protocol(e.to_string())),
        Some(SSE) => parse_sse(body, codec).map(ClientOutcome::Stream),
        _ => Err(net_http::Error::Protocol(
            "unsupported MCP response content type".into(),
        )),
    }
}
fn parse_sse(
    body: &[u8],
    codec: &Mutex<Cx>,
) -> std::result::Result<Vec<McpEnvelope>, net_http::Error> {
    let text =
        std::str::from_utf8(body).map_err(|_| net_http::Error::Protocol("non-UTF-8 SSE".into()))?;
    let mut out = Vec::new();
    for frame in text.split("\n\n") {
        let data = frame
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            out.push(
                decode(codec, data.into_bytes())
                    .map_err(|e| net_http::Error::Protocol(e.to_string()))?,
            );
        }
    }
    Ok(out)
}
