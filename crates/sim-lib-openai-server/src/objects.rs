use sim_citizen_derive::non_citizen;
use sim_kernel::{ContentId, Cx, Expr, Object, ObjectCompat, Result, Symbol};
use sim_lib_net_core::hex_encode;

/// Object-kind tag identifying a [`GatewayRequest`] in its `Expr` projection.
pub const GATEWAY_REQUEST_OBJECT: &str = "openai-gateway/request";
/// Object-kind tag identifying a [`GatewayResponse`] in its `Expr` projection.
pub const GATEWAY_RESPONSE_OBJECT: &str = "openai-gateway/response";
/// Object-kind tag identifying a [`GatewayRun`] in its `Expr` projection.
pub const GATEWAY_RUN_OBJECT: &str = "openai-gateway/run";
/// Object-kind tag identifying a [`GatewayEvent`] in its `Expr` projection.
pub const GATEWAY_EVENT_OBJECT: &str = "openai-gateway/event";

/// Represents an inbound HTTP request to the OpenAI-compatible gateway.
///
/// Carries the request line (method and path), headers, and raw body, plus
/// optional gateway-assigned metadata (an id and a receipt timestamp).
#[non_citizen(
    reason = "gateway request runtime shell; class-backed descriptor is openai/GatewayRequest",
    kind = "marker",
    descriptor = "openai/GatewayRequest"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayRequest {
    id: Option<String>,
    timestamp_ms: Option<u64>,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl GatewayRequest {
    /// Builds a request from a method, path, headers, and body, with no id or
    /// timestamp metadata attached yet.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            id: None,
            timestamp_ms: None,
            method: method.into(),
            path: path.into(),
            headers,
            body,
        }
    }

    /// Builds a `GET` request for the given path with empty headers and body.
    pub fn get(path: impl Into<String>) -> Self {
        Self::new("GET", path, Vec::new(), Vec::new())
    }

    /// Attaches a gateway-assigned id and receipt timestamp, returning the
    /// updated request.
    pub fn with_metadata(mut self, id: impl Into<String>, timestamp_ms: u64) -> Self {
        self.id = Some(id.into());
        self.timestamp_ms = Some(timestamp_ms);
        self
    }

    /// Returns the gateway-assigned id, or `None` if no metadata is attached.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Returns the receipt timestamp in milliseconds, or `None` if unset.
    pub fn timestamp_ms(&self) -> Option<u64> {
        self.timestamp_ms
    }

    /// Returns the HTTP method (for example `GET` or `POST`).
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the request path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the request headers as name/value pairs.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Returns the raw request body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Projects the request into its canonical `Expr` map representation.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("object", Expr::String(GATEWAY_REQUEST_OBJECT.to_owned())),
            optional_string_field("id", self.id.as_deref()),
            optional_u64_field("timestamp-ms", self.timestamp_ms),
            field("method", Expr::String(self.method.clone())),
            field("path", Expr::String(self.path.clone())),
            field("headers", headers_expr(&self.headers)),
            field("body", Expr::Bytes(self.body.clone())),
        ])
    }
}

impl Object for GatewayRequest {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!(
            "#<openai-gateway-request {} {}>",
            self.method, self.path
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayRequest {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.to_expr())
    }
}

/// Represents an HTTP response produced by the gateway.
///
/// Pairs a status code with response headers and a raw body. Constructors such
/// as [`GatewayResponse::json`], [`GatewayResponse::text`], and
/// [`GatewayResponse::sse`] preset the `Content-Type` header.
#[non_citizen(
    reason = "gateway response runtime shell; class-backed descriptor is openai/GatewayResponse",
    kind = "marker",
    descriptor = "openai/GatewayResponse"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl GatewayResponse {
    /// Builds a response from an explicit status, headers, and body.
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// Builds a response with `Content-Type: application/json`.
    pub fn json(status: u16, body: Vec<u8>) -> Self {
        Self::new(
            status,
            vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body,
        )
    }

    /// Builds a response with `Content-Type: text/plain`.
    pub fn text(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(
            status,
            vec![("Content-Type".to_owned(), "text/plain".to_owned())],
            body.into(),
        )
    }

    /// Builds a streaming response with `Content-Type: text/event-stream`.
    pub fn sse(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(
            status,
            vec![("Content-Type".to_owned(), "text/event-stream".to_owned())],
            body.into(),
        )
    }

    /// Returns the HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Returns the response headers as name/value pairs.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Returns the raw response body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns the first header value matching `name` case-insensitively, or
    /// `None` if no such header is present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Projects the response into its canonical `Expr` map representation.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("object", Expr::String(GATEWAY_RESPONSE_OBJECT.to_owned())),
            field("status", Expr::String(self.status.to_string())),
            field("headers", headers_expr(&self.headers)),
            field("body", Expr::Bytes(self.body.clone())),
        ])
    }
}

impl Object for GatewayResponse {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<openai-gateway-response {}>", self.status))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayResponse {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.to_expr())
    }
}

/// Represents a single gateway run: one accepted request being processed.
///
/// A run is keyed by its own id and the content id of the originating request,
/// tracks a lifecycle [`Symbol`] status (starting at `created`), and records
/// when it was created.
#[non_citizen(
    reason = "gateway run runtime shell; class-backed descriptor is openai/GatewayRun",
    kind = "marker",
    descriptor = "openai/GatewayRun"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayRun {
    id: String,
    request_content_id: ContentId,
    status: Symbol,
    created_at_ms: u64,
}

impl GatewayRun {
    /// Builds a run in the `created` state for the given id, originating
    /// request content id, and creation timestamp.
    pub fn new(id: impl Into<String>, request_content_id: ContentId, created_at_ms: u64) -> Self {
        Self {
            id: id.into(),
            request_content_id,
            status: Symbol::new("created"),
            created_at_ms,
        }
    }

    /// Returns the run with its lifecycle status replaced by `status`.
    pub fn with_status(mut self, status: impl Into<Symbol>) -> Self {
        self.status = status.into();
        self
    }

    /// Returns the run id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the content id of the request that initiated this run.
    pub fn request_content_id(&self) -> &ContentId {
        &self.request_content_id
    }

    /// Returns the current lifecycle status symbol.
    pub fn status(&self) -> &Symbol {
        &self.status
    }

    /// Returns the creation timestamp in milliseconds.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Projects the run into its canonical `Expr` map representation.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("object", Expr::String(GATEWAY_RUN_OBJECT.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field(
                "request-content-id",
                content_id_expr(&self.request_content_id),
            ),
            field("status", Expr::Symbol(self.status.clone())),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
        ])
    }
}

impl Object for GatewayRun {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<openai-gateway-run {}>", self.id))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayRun {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.to_expr())
    }
}

/// Represents one event emitted during a [`GatewayRun`].
///
/// Each event has its own id, the id of the run it belongs to, a monotonic
/// `sequence` within that run, a [`Symbol`] kind, an arbitrary `Expr` payload,
/// and a creation timestamp.
#[non_citizen(
    reason = "gateway event runtime shell; class-backed descriptor is openai/GatewayEvent",
    kind = "marker",
    descriptor = "openai/GatewayEvent"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayEvent {
    id: String,
    run_id: String,
    sequence: u64,
    kind: Symbol,
    payload: Expr,
    created_at_ms: u64,
}

impl GatewayEvent {
    /// Builds an event for a run from its id, parent run id, sequence number,
    /// kind, payload, and creation timestamp.
    pub fn new(
        id: impl Into<String>,
        run_id: impl Into<String>,
        sequence: u64,
        kind: impl Into<Symbol>,
        payload: Expr,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            run_id: run_id.into(),
            sequence,
            kind: kind.into(),
            payload,
            created_at_ms,
        }
    }

    /// Returns the event id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the id of the run this event belongs to.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns the event's sequence number within its run.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the event kind symbol.
    pub fn kind(&self) -> &Symbol {
        &self.kind
    }

    /// Returns the event payload expression.
    pub fn payload(&self) -> &Expr {
        &self.payload
    }

    /// Returns the creation timestamp in milliseconds.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Projects the event into its canonical `Expr` map representation.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("object", Expr::String(GATEWAY_EVENT_OBJECT.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("run-id", Expr::String(self.run_id.clone())),
            field("sequence", Expr::String(self.sequence.to_string())),
            field("event-kind", Expr::Symbol(self.kind.clone())),
            field("payload", self.payload.clone()),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
        ])
    }
}

impl Object for GatewayEvent {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!(
            "#<openai-gateway-event {} {}>",
            self.run_id, self.sequence
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayEvent {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.to_expr())
    }
}

/// Wraps a [`GatewayResponse`] as a runtime [`Object`] value.
///
/// This is the handle returned to the runtime; its serializable `Expr`
/// projection delegates to the wrapped response.
#[derive(Clone)]
#[non_citizen(
    reason = "runtime response wrapper; serializable projection is openai/GatewayResponse descriptor",
    kind = "handle",
    descriptor = "openai/GatewayResponse"
)]
pub struct GatewayResponseValue {
    response: GatewayResponse,
}

impl GatewayResponseValue {
    /// Wraps the given response as a runtime value.
    pub fn new(response: GatewayResponse) -> Self {
        Self { response }
    }

    /// Returns a reference to the wrapped response.
    pub fn response(&self) -> &GatewayResponse {
        &self.response
    }
}

impl Object for GatewayResponseValue {
    fn display(&self, cx: &mut Cx) -> Result<String> {
        self.response.display(cx)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayResponseValue {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.response.to_expr())
    }
}

/// Projects a [`ContentId`] into an `Expr` map with its algorithm, raw bytes,
/// and hex-encoded digest.
pub fn content_id_expr(id: &ContentId) -> Expr {
    Expr::Map(vec![
        field("algorithm", Expr::Symbol(id.algorithm.clone())),
        field("bytes", Expr::Bytes(id.bytes.to_vec())),
        field("hex", Expr::String(hex_encode(&id.bytes))),
    ])
}

/// Returns the lowercase hex encoding of a [`ContentId`]'s digest bytes.
pub fn content_id_hex(id: &ContentId) -> String {
    hex_encode(&id.bytes)
}

fn headers_expr(headers: &[(String, String)]) -> Expr {
    let mut sorted = headers.to_vec();
    sorted.sort_by_key(|(name, value)| (name.to_ascii_lowercase(), value.clone()));
    Expr::List(
        sorted
            .into_iter()
            .map(|(name, value)| {
                Expr::Map(vec![
                    field("name", Expr::String(name)),
                    field("value", Expr::String(value)),
                ])
            })
            .collect(),
    )
}

fn optional_string_field(name: &str, value: Option<&str>) -> (Expr, Expr) {
    field(
        name,
        value
            .map(|value| Expr::String(value.to_owned()))
            .unwrap_or(Expr::Nil),
    )
}

fn optional_u64_field(name: &str, value: Option<u64>) -> (Expr, Expr) {
    field(
        name,
        value
            .map(|value| Expr::String(value.to_string()))
            .unwrap_or(Expr::Nil),
    )
}

use sim_value::build::entry as field;
