use std::{collections::BTreeMap, fmt, time::Duration};

use serde_json::Value;
use sim_cancel::Cancellation;

use crate::{EndpointIdentity, SchemaContract};

/// MCP protocol family selected before application traffic.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Era {
    /// Final stateless protocol discovered with `server/discover`.
    Modern,
    /// Delivered initialize-era compatibility protocol.
    Legacy,
}

/// Validated immutable server discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    /// Selected protocol era.
    pub era: Era,
    /// Exact selected protocol version.
    pub version: String,
    /// Negotiated extension identifiers in canonical order.
    pub extensions: Vec<String>,
    /// Validated server implementation name.
    pub server_name: String,
    /// Validated server implementation version.
    pub server_version: String,
    /// Maximum discovery lifetime advertised by the server.
    pub ttl: Duration,
}

/// Binding-layer refusal with enough stable information for era classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingError {
    /// Deadline or transport timeout.
    Timeout,
    /// Child process exited; its endpoint identity is no longer valid.
    ProcessExited(i32),
    /// HTTP response, retaining the exact bounded body for pinned 400 classification.
    Http {
        /// Exact HTTP status.
        status: u16,
        /// Bounded response bytes.
        body: Vec<u8>,
    },
    /// JSON-RPC application error.
    Rpc {
        /// JSON-RPC error code.
        code: i64,
        /// JSON-RPC error message.
        message: String,
        /// Optional bounded error data.
        data: Option<Value>,
    },
    /// Malformed or incompatible response.
    Incompatible(String),
    /// Cooperative cancellation.
    Cancelled,
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("MCP binding timed out"),
            Self::ProcessExited(code) => write!(f, "MCP child exited with status {code}"),
            Self::Http { status, .. } => write!(f, "MCP HTTP status {status}"),
            Self::Rpc { code, message, .. } => write!(f, "MCP RPC error {code}: {message}"),
            Self::Incompatible(message) => write!(f, "incompatible MCP response: {message}"),
            Self::Cancelled => f.write_str("MCP request cancelled"),
        }
    }
}

impl std::error::Error for BindingError {}

/// One decoded binding response. Streaming bindings preserve frame order.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerReply {
    /// One complete result.
    Complete(Value),
    /// Modern request-more-time-to-respond result.
    InputRequired {
        /// Opaque request state copied exactly to the retry.
        request_state: Value,
        /// Declared input name to required capability.
        requested: BTreeMap<String, String>,
    },
    /// Subscription stream frames, including acknowledgement and terminal.
    Stream(Vec<Value>),
}

/// HTTP or stdio peer consumed by the transport-neutral state machine.
pub trait BindingPeer: Send + Sync {
    /// Stable endpoint identity. A process peer must change it for each child lifetime.
    fn endpoint(&self) -> EndpointIdentity;
    /// Binding kind used for the distinct pinned discovery classifier.
    fn binding_kind(&self) -> &'static str;
    /// Sends exactly one independently identified request.
    fn request(
        &self,
        era: Era,
        id: u64,
        method: &str,
        params: &Value,
        cancellation: &Cancellation,
        deadline_ms: u64,
    ) -> Result<PeerReply, BindingError>;
}

/// One imported invocation definition.
#[derive(Clone, Debug)]
pub struct Invocation {
    /// MCP method.
    pub method: String,
    /// Stable remote operation name or URI.
    pub operation: String,
    /// Input schema.
    pub input: SchemaContract,
    /// Output or content schema.
    pub output: SchemaContract,
    /// Whether the codec registry permits complete results to be cached.
    pub cache_eligible: bool,
    /// Whether the operation may have effects. Effects are never cached.
    pub effecting: bool,
}

/// Authenticated and bounded facts for one call.
pub struct CallContext<'a> {
    /// Stable authenticated principal/cache scope. Never a bearer token.
    pub principal_scope: &'a str,
    /// Capabilities the caller permits an MRTR broker to provide.
    pub input_capabilities: &'a [String],
    /// Canonical pagination cursor, when present.
    pub pagination_cursor: Option<&'a str>,
    /// Total deadline in monotonic host milliseconds.
    pub deadline_ms: u64,
    /// Current monotonic host milliseconds.
    pub now_ms: u64,
    /// Cooperative cancellation shared with the binding and broker.
    pub cancellation: &'a Cancellation,
}

/// One validated outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Complete semantic result.
    pub value: Value,
    /// Server TTL upper bound, if supplied by the complete result.
    pub ttl_ms: Option<u64>,
}

/// Validated request for host-owned interactive input.
pub struct InputRequest<'a> {
    /// Requested names and capabilities.
    pub requested: &'a BTreeMap<String, String>,
    /// Exact opaque continuation supplied by the server.
    pub request_state: &'a Value,
    /// Remaining total deadline.
    pub remaining_ms: u64,
}

/// Host-owned input acquisition. The client never discovers ambient authority.
pub trait InputBroker: Send + Sync {
    /// Obtains bounded answers to a validated request.
    fn acquire(&self, request: InputRequest<'_>) -> Result<BTreeMap<String, Value>, ClientError>;
}

/// Redaction-safe client ledger.
pub trait ClientLedger: Send + Sync {
    /// Records one phase without raw parameters, results, inputs, or credentials.
    fn record(&self, endpoint: &EndpointIdentity, method: &str, phase: &str);
}

/// Persistent cache controlled by host encryption and privacy policy.
pub trait PersistentCache: Send + Sync {
    /// Reads opaque JSON bytes for a canonical cache key.
    fn get(&self, key: &str, now_ms: u64) -> Result<Option<Vec<u8>>, ClientError>;
    /// Stores opaque JSON bytes until an absolute expiry.
    fn put(&self, key: &str, value: &[u8], expires_at_ms: u64) -> Result<(), ClientError>;
    /// Clears private entries when authenticated identity or token scope changes.
    fn clear_private(&self, endpoint: &EndpointIdentity) -> Result<(), ClientError>;
}

/// One verified subscription item.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    /// First frame.
    Acknowledged(String),
    /// Backpressured event delivered only after its id is checked.
    Event(Value),
    /// Dated terminal frame.
    Complete {
        /// Host-supplied completion instant.
        completed_at_ms: u64,
        /// Whether cooperative cancellation terminated the stream.
        cancelled: bool,
    },
}

/// Complete validated subscription sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    /// Exact subscription identifier.
    pub id: String,
    /// Acknowledgement, events, and dated terminal in wire order.
    pub events: Vec<ClientEvent>,
}

/// Client refusal.
#[derive(Debug)]
pub enum ClientError {
    /// Binding failure.
    Binding(BindingError),
    /// Protocol or policy refusal.
    Policy(String),
    /// Schema refusal.
    Schema(String),
    /// Discovery cannot be classified as either delivered era.
    UnrecognizedDiscovery,
    /// MRTR limits or capabilities were violated.
    InputRequired(String),
    /// Subscription sequence was invalid.
    Subscription(String),
    /// Cache organ failed.
    Cache(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding(error) => error.fmt(f),
            Self::Policy(message) => write!(f, "MCP client policy: {message}"),
            Self::Schema(message) => write!(f, "MCP schema: {message}"),
            Self::UnrecognizedDiscovery => f.write_str("unrecognized MCP discovery response"),
            Self::InputRequired(message) => write!(f, "MCP input_required: {message}"),
            Self::Subscription(message) => write!(f, "MCP subscription: {message}"),
            Self::Cache(message) => write!(f, "MCP cache: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}
impl From<BindingError> for ClientError {
    fn from(value: BindingError) -> Self {
        Self::Binding(value)
    }
}
