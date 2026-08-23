//! Final-protocol Streamable HTTP adapter for stateless MCP.
//!
//! This crate owns endpoint and projection policy. Parsing, sockets, TLS,
//! generic body bounds, and backpressure stay in the composed server and
//! client organs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{io, sync::Mutex, time::Instant};

use sim_cancel::Cancellation;
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_mcp::{
    McpEnvelope, McpError, McpErrorEnvelope, PARSE_ERROR, envelope_to_expr, expr_to_envelope,
};
use sim_kernel::{Cx, EncodeOptions, Error, Expr, ReadPolicy, Result, Symbol};
use sim_lib_mcp::{CachePolicy, McpService, NegotiatedExtensions, Principal, RequestContext};
use sim_lib_mcp_legacy::LegacyConnection;
use sim_lib_net_http as net_http;
use sim_lib_server::{
    BodyReader, RawHandler, RequestHead, RequestScope, ResponseHead, ResponseWriter,
};

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

const JSON: &str = "application/json";
const SSE: &str = "text/event-stream";
const PROTOCOL: &str = "2026-07-28";

/// Origin policy applied before a request body is read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OriginPolicy {
    /// Only requests without an Origin header are accepted. This is valid only
    /// for an explicitly loopback-only service.
    LoopbackOnly,
    /// Exact serialized origins accepted by a browser-reachable service.
    Exact(Vec<String>),
}

impl OriginPolicy {
    fn admits(&self, origins: &[&str]) -> bool {
        match self {
            Self::LoopbackOnly => origins.is_empty(),
            Self::Exact(allowed) => origins.len() == 1 && allowed.iter().any(|v| v == origins[0]),
        }
    }
}

/// Immutable policy for one modern endpoint and optional explicit legacy endpoint.
#[derive(Clone, Debug)]
pub struct ServerPolicy {
    /// Exact modern request target.
    pub endpoint: String,
    /// Optional exact initialize-era target; no era is inferred from history.
    pub legacy_endpoint: Option<String>,
    /// Pre-body Origin policy.
    pub origins: OriginPolicy,
    /// Maximum body bytes accumulated by this protocol boundary.
    pub max_body_bytes: usize,
    /// Maximum number of bounded comment keep-alives between messages.
    pub max_keepalives: usize,
}

impl ServerPolicy {
    /// Validates and constructs server policy.
    pub fn new(
        endpoint: impl Into<String>,
        origins: OriginPolicy,
        max_body_bytes: usize,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        if !endpoint.starts_with('/') || max_body_bytes == 0 {
            return Err(Error::Eval(
                "MCP HTTP endpoint must be absolute and body cap non-zero".into(),
            ));
        }
        if matches!(&origins, OriginPolicy::Exact(values) if values.is_empty()) {
            return Err(Error::Eval(
                "browser-reachable MCP HTTP requires an exact origin".into(),
            ));
        }
        Ok(Self {
            endpoint,
            legacy_endpoint: None,
            origins,
            max_body_bytes,
            max_keepalives: 1,
        })
    }

    /// Adds a distinct explicit legacy endpoint.
    pub fn with_legacy_endpoint(mut self, endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        if !endpoint.starts_with('/') || endpoint == self.endpoint {
            return Err(Error::Eval(
                "legacy endpoint must be distinct and absolute".into(),
            ));
        }
        self.legacy_endpoint = Some(endpoint);
        Ok(self)
    }
}

/// Authenticated request facts supplied by the HTTP host.
#[derive(Clone, Debug)]
pub struct RequestIdentity {
    /// Authenticated caller.
    pub principal: Principal,
}

/// Host policy that authenticates a request without granting ambient authority.
pub trait IdentityProvider: Send + Sync {
    /// Resolves one request identity from already-parsed request facts.
    fn identify(&self, head: &RequestHead) -> Result<RequestIdentity>;
}

/// Boundary implemented by the modern service and explicit legacy adapters.
pub trait McpDispatch: Send + Sync {
    /// Dispatches one checked envelope with request-owned context and cancellation.
    fn dispatch(
        &self,
        context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>>;
}

/// Concrete dispatcher for the immutable modern [`McpService`].
pub struct ServiceDispatch {
    service: McpService,
    host_seed: Mutex<Cx>,
}

impl ServiceDispatch {
    /// Binds the service to an explicitly prepared host context seed.
    pub fn new(service: McpService, host_seed: Cx) -> Self {
        Self {
            service,
            host_seed: Mutex::new(host_seed),
        }
    }
}

impl McpDispatch for ServiceDispatch {
    fn dispatch(
        &self,
        context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        if cancellation.is_cancelled() {
            return Err(Error::Eval("MCP HTTP request cancelled".into()));
        }
        match envelope {
            McpEnvelope::Request(request) => {
                let mut host_seed = self
                    .host_seed
                    .lock()
                    .map_err(|_| Error::PoisonedLock("MCP service seed"))?;
                self.service
                    .handle(&mut host_seed, context, request)
                    .map(Iterator::collect)
            }
            McpEnvelope::Notification(_) => Ok(Vec::new()),
            _ => Err(Error::Eval(
                "MCP HTTP dispatch accepts only requests and notifications".into(),
            )),
        }
    }
}

/// Explicit initialize-era dispatcher for a distinct legacy endpoint.
pub struct LegacyDispatch {
    connection: Mutex<(LegacyConnection, Cx)>,
}
impl LegacyDispatch {
    /// Binds one intentionally stateful compatibility connection and its host context.
    pub fn new(connection: LegacyConnection, host_seed: Cx) -> Self {
        Self {
            connection: Mutex::new((connection, host_seed)),
        }
    }
}
impl McpDispatch for LegacyDispatch {
    fn dispatch(
        &self,
        _context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        if cancellation.is_cancelled() {
            return Err(Error::Eval("legacy MCP HTTP request cancelled".into()));
        }
        let mut state = self
            .connection
            .lock()
            .map_err(|_| Error::PoisonedLock("legacy MCP connection"))?;
        let (connection, cx) = &mut *state;
        connection.handle_envelope(cx, envelope)
    }
}

/// Clock projection used only to date forbidden responses and bound SSE comments.
pub trait HttpClock: Send + Sync {
    /// Current RFC 9110 HTTP-date text.
    fn http_date(&self) -> String;
    /// One monotonic keep-alive sequence number, if a comment is due.
    fn keepalive(&self) -> Option<u64>;
}

/// Streamable HTTP request handler over the shared raw server seam.
pub struct McpHttpHandler<D, I, K> {
    policy: ServerPolicy,
    modern: D,
    legacy: Option<Box<dyn McpDispatch>>,
    identity: I,
    clock: K,
    codec: Mutex<Cx>,
}

impl<D, I, K> McpHttpHandler<D, I, K>
where
    D: McpDispatch,
    I: IdentityProvider,
    K: HttpClock,
{
    /// Creates a modern-only handler. `codec_cx` must have `codec:mcp` loaded.
    pub fn new(policy: ServerPolicy, modern: D, identity: I, clock: K, codec_cx: Cx) -> Self {
        Self {
            policy,
            modern,
            legacy: None,
            identity,
            clock,
            codec: Mutex::new(codec_cx),
        }
    }

    /// Attaches an explicit legacy dispatcher to the configured legacy endpoint.
    pub fn with_legacy(mut self, legacy: impl McpDispatch + 'static) -> Result<Self> {
        if self.policy.legacy_endpoint.is_none() {
            return Err(Error::Eval(
                "legacy dispatcher requires an explicit endpoint".into(),
            ));
        }
        self.legacy = Some(Box::new(legacy));
        Ok(self)
    }
}

impl<D, I, K> RawHandler for McpHttpHandler<D, I, K>
where
    D: McpDispatch,
    I: IdentityProvider,
    K: HttpClock,
{
    fn handle(
        &self,
        head: &RequestHead,
        body: &mut dyn BodyReader,
        response: &mut dyn ResponseWriter,
        scope: &RequestScope,
    ) -> Result<()> {
        let endpoint = target_path(&head.target);
        let dispatch: &dyn McpDispatch = if endpoint == self.policy.endpoint {
            &self.modern
        } else if self.policy.legacy_endpoint.as_deref() == Some(endpoint) {
            self.legacy
                .as_deref()
                .ok_or_else(|| Error::Eval("legacy endpoint unavailable".into()))?
        } else {
            return empty(
                response,
                scope,
                404,
                common_headers(&self.clock.http_date()),
            );
        };
        if head.method != "POST" {
            let mut headers = common_headers(&self.clock.http_date());
            headers.push(("Allow".into(), "POST".into()));
            return empty(response, scope, 405, headers);
        }
        let origins = header_values(head, "origin");
        if !self.policy.origins.admits(&origins) {
            return empty(
                response,
                scope,
                403,
                common_headers(&self.clock.http_date()),
            );
        }
        if unique_header(head, "content-type")
            .ok()
            .flatten()
            .is_none_or(|v| media_type(v) != JSON)
        {
            return protocol_error(
                response,
                scope,
                400,
                "content-type must be application/json",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let accept = match unique_header(head, "accept") {
            Ok(Some(value)) if accepts(value, JSON) || accepts(value, SSE) => value,
            _ => {
                return protocol_error(
                    response,
                    scope,
                    400,
                    "accept must include application/json or text/event-stream",
                    &self.clock.http_date(),
                    &self.codec,
                );
            }
        };
        if duplicate_mcp_headers(head) {
            return protocol_error(
                response,
                scope,
                400,
                "duplicate MCP projection header",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let bytes = read_body(body, scope, self.policy.max_body_bytes)
            .map_err(|e| Error::HostError(e.to_string()))?;
        let envelope = match decode(&self.codec, bytes) {
            Ok(value) => value,
            Err(error) => {
                return protocol_error(
                    response,
                    scope,
                    400,
                    &error.to_string(),
                    &self.clock.http_date(),
                    &self.codec,
                );
            }
        };
        if let Err(message) = check_projection(head, &envelope) {
            return protocol_error(
                response,
                scope,
                400,
                message,
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let identity = self.identity.identify(head)?;
        let request_id = request_id(&envelope);
        let context = RequestContext::new(
            request_id,
            PROTOCOL,
            NegotiatedExtensions::none(),
            identity.principal,
            CachePolicy::Bypass,
        );
        let replies = dispatch.dispatch(&context, scope.cancellation(), envelope.clone())?;
        if matches!(envelope, McpEnvelope::Notification(_)) {
            if !replies.is_empty() {
                return Err(Error::Eval(
                    "notification dispatcher emitted a response".into(),
                ));
            }
            return empty(
                response,
                scope,
                202,
                common_headers(&self.clock.http_date()),
            );
        }
        if accepts(accept, SSE) {
            return write_sse(
                response,
                scope,
                &replies,
                &self.clock,
                self.policy.max_keepalives,
                &self.codec,
            );
        }
        if replies.len() != 1 {
            return protocol_error(
                response,
                scope,
                400,
                "JSON response requires exactly one message",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let bytes = encode(&self.codec, &replies[0])?;
        let mut headers = common_headers(&self.clock.http_date());
        headers.push(("Content-Type".into(), JSON.into()));
        response
            .write_head(
                ResponseHead {
                    status: 200,
                    headers,
                },
                scope,
            )
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
        response
            .write_chunk(&bytes, scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
        response
            .finish(&[], scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)
    }
}

fn target_path(target: &str) -> &str {
    target.split('?').next().unwrap_or(target)
}
fn media_type(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}
fn accepts(value: &str, wanted: &str) -> bool {
    value
        .split(',')
        .any(|v| media_type(v).eq_ignore_ascii_case(wanted) || media_type(v) == "*/*")
}
fn header_values<'a>(head: &'a RequestHead, name: &str) -> Vec<&'a str> {
    head.headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect()
}
fn unique_header<'a>(
    head: &'a RequestHead,
    name: &str,
) -> std::result::Result<Option<&'a str>, ()> {
    let v = header_values(head, name);
    if v.len() > 1 {
        Err(())
    } else {
        Ok(v.first().copied())
    }
}
fn duplicate_mcp_headers(head: &RequestHead) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    head.headers
        .iter()
        .filter(|(n, _)| {
            n.to_ascii_lowercase().starts_with("mcp-")
                || n.to_ascii_lowercase().starts_with("x-mcp-")
        })
        .any(|(n, _)| !seen.insert(n.to_ascii_lowercase()))
}
fn read_body(body: &mut dyn BodyReader, scope: &RequestScope, cap: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = body.next_chunk(scope)? {
        if out.len().saturating_add(chunk.len()) > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP body cap exceeded",
            ));
        }
        out.extend(chunk);
    }
    Ok(out)
}
fn request_id(envelope: &McpEnvelope) -> String {
    match envelope {
        McpEnvelope::Request(r) => format!("{:?}", r.id),
        McpEnvelope::Notification(n) => format!("notify:{}", n.method),
        _ => "invalid".into(),
    }
}

fn check_projection(
    head: &RequestHead,
    envelope: &McpEnvelope,
) -> std::result::Result<(), &'static str> {
    let (method, params) = match envelope {
        McpEnvelope::Request(r) => (r.method.as_str(), &r.params),
        McpEnvelope::Notification(n) => (n.method.as_str(), &n.params),
        _ => return Err("request body must be a request or notification"),
    };
    if unique_header(head, "mcp-protocol-version").map_err(|_| "duplicate protocol header")?
        != Some(PROTOCOL)
    {
        return Err("protocol header/body mismatch");
    }
    if let Some(projected) =
        unique_header(head, "mcp-method").map_err(|_| "duplicate method header")?
    {
        if projected != method {
            return Err("method header/body mismatch");
        }
    }
    let body_name = expr_field(params, "name");
    if let Some(projected) = unique_header(head, "mcp-name").map_err(|_| "duplicate name header")? {
        if body_name != Some(projected) {
            return Err("name header/body mismatch");
        }
    }
    if let Some(body_version) = expr_field(params, "protocolVersion") {
        if body_version != PROTOCOL {
            return Err("protocol header/body mismatch");
        }
    }
    for (name, value) in &head.headers {
        if let Some(parameter) = name.to_ascii_lowercase().strip_prefix("mcp-parameter-") {
            if expr_field(params, parameter) != Some(value.as_str()) {
                return Err("parameter header/body mismatch");
            }
        }
    }
    for (name, value) in &head.headers {
        if name.to_ascii_lowercase().starts_with("x-mcp-") && !safe_projection(value) {
            return Err("unsafe MCP sentinel header");
        }
    }
    Ok(())
}
fn expr_field<'a>(expr: &'a Expr, wanted: &str) -> Option<&'a str> {
    match sim_value::access::field_any(expr, wanted) {
        Some(Expr::String(value)) => Some(value),
        _ => None,
    }
}
fn safe_projection(value: &str) -> bool {
    value.strip_prefix(":base64:").map_or_else(
        || !value.is_empty() && value.bytes().all(|b| (0x21..=0x7e).contains(&b)),
        |v| {
            !v.is_empty()
                && v.len() % 4 != 1
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        },
    )
}
fn common_headers(date: &str) -> Vec<(String, String)> {
    vec![
        ("Date".into(), date.into()),
        ("Cache-Control".into(), "no-store, private".into()),
        ("Vary".into(), "Origin, Accept, Authorization".into()),
        ("X-Content-Type-Options".into(), "nosniff".into()),
    ]
}
fn empty(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    status: u16,
    headers: Vec<(String, String)>,
) -> Result<()> {
    response
        .write_head(ResponseHead { status, headers }, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn protocol_error(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    status: u16,
    message: &str,
    date: &str,
    codec: &Mutex<Cx>,
) -> Result<()> {
    let envelope = McpEnvelope::Error(McpErrorEnvelope {
        id: Expr::Nil,
        error: McpError {
            code: PARSE_ERROR,
            message: message.into(),
            data: Expr::Nil,
        },
    });
    let bytes = encode(codec, &envelope)?;
    let mut headers = common_headers(date);
    headers.push(("Content-Type".into(), JSON.into()));
    response
        .write_head(ResponseHead { status, headers }, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .write_chunk(&bytes, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn write_sse(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    replies: &[McpEnvelope],
    clock: &dyn HttpClock,
    keepalive_cap: usize,
    codec: &Mutex<Cx>,
) -> Result<()> {
    let mut headers = common_headers(&clock.http_date());
    headers.extend([
        ("Content-Type".into(), SSE.into()),
        ("X-Accel-Buffering".into(), "no".into()),
        ("Content-Encoding".into(), "identity".into()),
    ]);
    response
        .write_head(
            ResponseHead {
                status: 200,
                headers,
            },
            scope,
        )
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    for (index, reply) in replies.iter().enumerate() {
        if index < keepalive_cap {
            if let Some(tick) = clock.keepalive() {
                response
                    .write_chunk(format!(": keepalive {tick}\n\n").as_bytes(), scope)
                    .inspect_err(|_| scope.cancel_peer_drop())
                    .map_err(Error::host_io)?;
            }
        }
        let payload = encode(codec, reply)?;
        let mut frame = b"event: message\ndata: ".to_vec();
        frame.extend(payload);
        frame.extend(b"\n\n");
        response
            .write_chunk(&frame, scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
    }
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn decode(codec: &Mutex<Cx>, bytes: Vec<u8>) -> Result<McpEnvelope> {
    let mut codec = codec
        .lock()
        .map_err(|_| Error::PoisonedLock("MCP HTTP codec"))?;
    let expr = decode_with_codec(
        &mut codec,
        &Symbol::qualified("codec", "mcp"),
        Input::Bytes(bytes),
        ReadPolicy::default(),
    )?;
    expr_to_envelope(&expr)
}
fn encode(codec: &Mutex<Cx>, envelope: &McpEnvelope) -> Result<Vec<u8>> {
    let mut codec = codec
        .lock()
        .map_err(|_| Error::PoisonedLock("MCP HTTP codec"))?;
    encode_with_codec(
        &mut codec,
        &Symbol::qualified("codec", "mcp"),
        &envelope_to_expr(envelope),
        EncodeOptions::default(),
    )?
    .into_text()
    .map(String::into_bytes)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use sim_codec_mcp::{McpCodecLib, McpRequest, McpResponse};
    use sim_kernel::{DefaultFactory, EagerPolicy};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Identity;
    impl IdentityProvider for Identity {
        fn identify(&self, _head: &RequestHead) -> Result<RequestIdentity> {
            Ok(RequestIdentity {
                principal: Principal::new("test-principal"),
            })
        }
    }
    struct Clock;
    impl HttpClock for Clock {
        fn http_date(&self) -> String {
            "Sun, 23 Aug 2026 12:00:00 GMT".into()
        }
        fn keepalive(&self) -> Option<u64> {
            Some(7)
        }
    }
    struct Dispatch {
        calls: Arc<AtomicUsize>,
    }
    impl McpDispatch for Dispatch {
        fn dispatch(
            &self,
            context: &RequestContext,
            cancellation: &Cancellation,
            envelope: McpEnvelope,
        ) -> Result<Vec<McpEnvelope>> {
            assert_eq!(context.principal().subject(), "test-principal");
            assert!(!cancellation.is_cancelled());
            self.calls.fetch_add(1, Ordering::SeqCst);
            match envelope {
                McpEnvelope::Request(request) => Ok(vec![McpEnvelope::Response(McpResponse {
                    id: request.id,
                    result: Expr::String("typed".into()),
                })]),
                McpEnvelope::Notification(_) => Ok(Vec::new()),
                _ => unreachable!(),
            }
        }
    }
    struct Body {
        chunks: Vec<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }
    impl BodyReader for Body {
        fn next_chunk(&mut self, _scope: &RequestScope) -> io::Result<Option<Vec<u8>>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(if self.chunks.is_empty() {
                None
            } else {
                Some(self.chunks.remove(0))
            })
        }
    }
    #[derive(Default)]
    struct Writer {
        head: Option<ResponseHead>,
        chunks: Vec<Vec<u8>>,
        fail_after_head: bool,
    }
    impl ResponseWriter for Writer {
        fn write_head(&mut self, head: ResponseHead, _scope: &RequestScope) -> io::Result<()> {
            self.head = Some(head);
            Ok(())
        }
        fn write_chunk(&mut self, chunk: &[u8], _scope: &RequestScope) -> io::Result<()> {
            if self.fail_after_head {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "peer drop"));
            }
            self.chunks.push(chunk.to_vec());
            Ok(())
        }
        fn finish(
            &mut self,
            _trailers: &[(String, String)],
            _scope: &RequestScope,
        ) -> io::Result<()> {
            Ok(())
        }
    }
    fn codec_cx() -> Cx {
        let mut cx = Cx::new(
            Arc::new(EagerPolicy),
            Arc::new(DefaultFactory),
            sim_kernel::HandleSeed::new(0x4d43_5048),
        );
        let codec = McpCodecLib::new(cx.registry_mut().fresh_codec_id());
        cx.load_lib(&codec).unwrap();
        cx
    }
    fn request(method: &str) -> McpEnvelope {
        McpEnvelope::Request(McpRequest {
            id: Expr::String("1".into()),
            method: method.into(),
            params: Expr::Nil,
        })
    }
    fn bytes(envelope: &McpEnvelope) -> Vec<u8> {
        encode(&Mutex::new(codec_cx()), envelope).unwrap()
    }
    fn head(method: &str, accept: &str) -> RequestHead {
        RequestHead {
            method: method.into(),
            target: "/mcp".into(),
            headers: vec![
                ("Content-Type".into(), JSON.into()),
                ("Accept".into(), accept.into()),
                ("MCP-Protocol-Version".into(), PROTOCOL.into()),
                ("MCP-Method".into(), "ping".into()),
            ],
            peer: Some("127.0.0.1:1".into()),
            local: Some("127.0.0.1:2".into()),
        }
    }
    fn handler(calls: Arc<AtomicUsize>) -> McpHttpHandler<Dispatch, Identity, Clock> {
        McpHttpHandler::new(
            ServerPolicy::new("/mcp", OriginPolicy::LoopbackOnly, 4096).unwrap(),
            Dispatch { calls },
            Identity,
            Clock,
            codec_cx(),
        )
    }
    fn scope() -> RequestScope {
        RequestScope::child(&Cancellation::new(), std::time::Duration::from_secs(1))
    }

    #[test]
    fn method_and_origin_reject_before_body_read() {
        for (mut request, status) in [
            (head("GET", JSON), 405),
            (
                RequestHead {
                    headers: vec![("Origin".into(), "https://evil.example".into())],
                    ..head("POST", JSON)
                },
                403,
            ),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let reads = Arc::new(AtomicUsize::new(0));
            let mut body = Body {
                chunks: vec![vec![1]],
                reads: reads.clone(),
            };
            let mut writer = Writer::default();
            handler(calls.clone())
                .handle(&request, &mut body, &mut writer, &scope())
                .unwrap();
            assert_eq!(writer.head.unwrap().status, status);
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            request.target.clear();
        }
    }
    #[test]
    fn json_and_sse_have_same_typed_result_and_no_session_headers() {
        for accept in [JSON, SSE] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut body = Body {
                chunks: vec![bytes(&request("ping"))],
                reads: Arc::new(AtomicUsize::new(0)),
            };
            let mut writer = Writer::default();
            handler(calls.clone())
                .handle(&head("POST", accept), &mut body, &mut writer, &scope())
                .unwrap();
            assert_eq!(writer.head.as_ref().unwrap().status, 200);
            assert!(
                !writer
                    .head
                    .as_ref()
                    .unwrap()
                    .headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("mcp-session-id"))
            );
            let wire = writer.chunks.concat();
            let decoded = if accept == JSON {
                decode(&Mutex::new(codec_cx()), wire).unwrap()
            } else {
                parse_sse(&wire, &Mutex::new(codec_cx()))
                    .unwrap()
                    .pop()
                    .unwrap()
            };
            let McpEnvelope::Response(response) = decoded else {
                panic!()
            };
            assert_eq!(response.result, Expr::String("typed".into()));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn notification_is_202_and_projection_conflict_never_dispatches() {
        let notification = McpEnvelope::Notification(sim_codec_mcp::McpNotification {
            method: "changed".into(),
            params: Expr::Nil,
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let mut h = head("POST", JSON);
        h.headers
            .iter_mut()
            .find(|(n, _)| n == "MCP-Method")
            .unwrap()
            .1 = "changed".into();
        let mut body = Body {
            chunks: vec![bytes(&notification)],
            reads: Arc::new(AtomicUsize::new(0)),
        };
        let mut writer = Writer::default();
        handler(calls.clone())
            .handle(&h, &mut body, &mut writer, &scope())
            .unwrap();
        assert_eq!(writer.head.unwrap().status, 202);
        assert!(writer.chunks.is_empty());
        let mut conflict = head("POST", JSON);
        conflict.headers.push(("mcp-method".into(), "other".into()));
        let mut body = Body {
            chunks: vec![bytes(&request("ping"))],
            reads: Arc::new(AtomicUsize::new(0)),
        };
        let mut writer = Writer::default();
        handler(calls.clone())
            .handle(&conflict, &mut body, &mut writer, &scope())
            .unwrap();
        assert_eq!(writer.head.unwrap().status, 400);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn response_drop_cancels_only_its_scope() {
        let calls = Arc::new(AtomicUsize::new(0));
        let parent = Cancellation::new();
        let first = RequestScope::child(&parent, std::time::Duration::from_secs(1));
        let second = RequestScope::child(&parent, std::time::Duration::from_secs(1));
        let mut body = Body {
            chunks: vec![bytes(&request("ping"))],
            reads: Arc::new(AtomicUsize::new(0)),
        };
        let mut writer = Writer {
            fail_after_head: true,
            ..Default::default()
        };
        assert!(
            handler(calls)
                .handle(&head("POST", JSON), &mut body, &mut writer, &first)
                .is_err()
        );
        assert!(first.cancellation().is_cancelled());
        assert!(!second.cancellation().is_cancelled());
    }
}
