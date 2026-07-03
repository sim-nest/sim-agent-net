use std::collections::BTreeSet;

use sim_kernel::{CapabilityName, Expr};
#[cfg(feature = "stream")]
use sim_lib_stream_core::StreamPacket;

use crate::{McpNativeCard, McpProfile};

/// MCP protocol version advertised by a freshly created [`McpSession`].
pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpBoundaryLimit {
    Deadline,
    Rate,
    ActiveRequests,
}

/// Mutable per-connection MCP session state.
///
/// Tracks the handshake, the visibility profile, granted capabilities, and the
/// in-flight request bookkeeping used to enforce deadline, rate, and
/// concurrency boundaries.
#[derive(Clone)]
pub struct McpSession {
    /// Stable session identifier.
    pub id: String,
    /// Whether the `initialize` handshake has completed.
    pub initialized: bool,
    /// Client info reported during initialization, if any.
    pub client_info: Option<Expr>,
    /// Negotiated MCP protocol version.
    pub protocol_version: String,
    /// Visibility profile filtering the surface for this session.
    pub profile: McpProfile,
    /// Native cards exposed through this session.
    pub native_cards: Vec<McpNativeCard>,
    /// Capabilities granted to this session.
    pub granted_capabilities: Vec<CapabilityName>,
    /// Optional per-request deadline budget, in milliseconds.
    pub deadline_ms: Option<u64>,
    /// Optional cap on the total number of requests admitted.
    pub rate_limit: Option<usize>,
    /// Optional cap on concurrently active requests.
    pub active_request_limit: Option<usize>,
    requests_seen: usize,
    /// Identifiers of currently in-flight requests.
    pub active_requests: BTreeSet<String>,
    #[cfg(feature = "cassette")]
    pub(crate) cassette: Option<crate::McpCassette>,
    #[cfg(feature = "stream")]
    pub(crate) cancelled_requests: BTreeSet<String>,
    #[cfg(feature = "stream")]
    stream_packets: Vec<StreamPacket>,
    /// Whether the peer has requested shutdown.
    pub shutdown_requested: bool,
}

impl McpSession {
    /// Creates a session with the given `id` and visibility `profile`.
    pub fn new(id: impl Into<String>, profile: McpProfile) -> Self {
        Self {
            id: id.into(),
            initialized: false,
            client_info: None,
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_owned(),
            profile,
            native_cards: Vec::new(),
            granted_capabilities: Vec::new(),
            deadline_ms: None,
            rate_limit: None,
            active_request_limit: None,
            requests_seen: 0,
            active_requests: BTreeSet::new(),
            #[cfg(feature = "cassette")]
            cassette: None,
            #[cfg(feature = "stream")]
            cancelled_requests: BTreeSet::new(),
            #[cfg(feature = "stream")]
            stream_packets: Vec::new(),
            shutdown_requested: false,
        }
    }

    /// Creates a permissive session for tests and fixtures.
    pub fn fixture() -> Self {
        Self::new("fixture", McpProfile::all())
    }

    /// Returns the session with its native cards replaced by `cards`.
    pub fn with_native_cards(mut self, cards: Vec<McpNativeCard>) -> Self {
        self.native_cards = cards;
        self
    }

    /// Returns the session with `capability` added to the granted set.
    pub fn with_granted_capability(mut self, capability: CapabilityName) -> Self {
        self.granted_capabilities.push(capability);
        self
    }

    /// Returns the session with a per-request `deadline_ms` budget.
    pub fn with_deadline_ms(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }

    /// Returns the session with a total request `limit`.
    pub fn with_rate_limit(mut self, limit: usize) -> Self {
        self.rate_limit = Some(limit);
        self
    }

    /// Returns the session with a concurrent active-request `limit`.
    pub fn with_active_request_limit(mut self, limit: usize) -> Self {
        self.active_request_limit = Some(limit);
        self
    }

    /// Returns the session with a recording/replay `cassette` attached.
    #[cfg(feature = "cassette")]
    pub fn with_cassette(mut self, cassette: crate::McpCassette) -> Self {
        self.cassette = Some(cassette);
        self
    }

    /// Returns the attached cassette, if any.
    #[cfg(feature = "cassette")]
    pub fn cassette(&self) -> Option<&crate::McpCassette> {
        self.cassette.as_ref()
    }

    /// Returns a mutable reference to the attached cassette, if any.
    #[cfg(feature = "cassette")]
    pub fn cassette_mut(&mut self) -> Option<&mut crate::McpCassette> {
        self.cassette.as_mut()
    }

    pub(crate) fn admit_request(&mut self, id: &Expr) -> std::result::Result<(), McpBoundaryLimit> {
        if self.deadline_ms == Some(0) {
            return Err(McpBoundaryLimit::Deadline);
        }
        if self
            .active_request_limit
            .is_some_and(|limit| self.active_requests.len() >= limit)
        {
            return Err(McpBoundaryLimit::ActiveRequests);
        }
        if self
            .rate_limit
            .is_some_and(|limit| self.requests_seen >= limit)
        {
            return Err(McpBoundaryLimit::Rate);
        }
        self.requests_seen += 1;
        self.begin_request(id);
        Ok(())
    }

    pub(crate) fn begin_request(&mut self, id: &Expr) {
        self.active_requests.insert(request_key(id));
    }

    pub(crate) fn end_request(&mut self, id: &Expr) {
        let key = request_key(id);
        self.active_requests.remove(&key);
        #[cfg(feature = "stream")]
        self.cancelled_requests.remove(&key);
    }

    #[cfg(feature = "stream")]
    pub(crate) fn request_is_active(&self, id: &Expr) -> bool {
        self.active_requests.contains(&request_key(id))
    }

    #[cfg(feature = "stream")]
    pub(crate) fn mark_request_cancelled(&mut self, id: &Expr) {
        self.cancelled_requests.insert(request_key(id));
    }

    /// Reports whether the request identified by `id` has been cancelled.
    #[cfg(feature = "stream")]
    pub fn request_cancelled(&self, id: &Expr) -> bool {
        self.cancelled_requests.contains(&request_key(id))
    }

    #[cfg(feature = "stream")]
    pub(crate) fn record_stream_packet(&mut self, packet: StreamPacket) {
        self.stream_packets.push(packet);
    }

    #[cfg(feature = "stream")]
    pub(crate) fn record_stream_packets(&mut self, packets: Vec<StreamPacket>) {
        self.stream_packets.extend(packets);
    }

    /// Returns the stream packets recorded during this session.
    #[cfg(feature = "stream")]
    pub fn stream_packets(&self) -> &[StreamPacket] {
        &self.stream_packets
    }
}

fn request_key(id: &Expr) -> String {
    match id {
        Expr::String(value) => value.clone(),
        Expr::Number(number) => format!("{}:{}", number.domain, number.canonical),
        Expr::Nil => "nil".to_owned(),
        _ => format!("{id:?}"),
    }
}
