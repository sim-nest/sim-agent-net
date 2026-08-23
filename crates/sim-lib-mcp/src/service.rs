//! Stateless MCP service boundary.
//!
//! Connection negotiation belongs to adapters. The service owns only an
//! immutable description and receives every request fact explicitly.

use std::collections::{BTreeMap, BTreeSet};

use sim_codec_mcp::{METHOD_NOT_FOUND, McpEnvelope, McpError, McpErrorEnvelope, McpRequest};
use sim_kernel::{CapabilityName, Cx, Expr, Result};

use crate::{McpNativeCard, McpProfile, McpRouter, McpSession};

/// Immutable description of one MCP service.
#[derive(Clone)]
pub struct ServerDescription {
    name: String,
    version: String,
    profile: McpProfile,
    native_cards: Vec<McpNativeCard>,
    granted_capabilities: Vec<CapabilityName>,
}

impl ServerDescription {
    /// Creates a service description with no cards or ambient grants.
    pub fn new(name: impl Into<String>, version: impl Into<String>, profile: McpProfile) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            profile,
            native_cards: Vec::new(),
            granted_capabilities: Vec::new(),
        }
    }

    /// Replaces the native cards projected by the service.
    pub fn with_native_cards(mut self, cards: Vec<McpNativeCard>) -> Self {
        self.native_cards = cards;
        self
    }

    /// Adds an explicitly configured service capability.
    pub fn with_granted_capability(mut self, capability: CapabilityName) -> Self {
        self.granted_capabilities.push(capability);
        self
    }

    /// Stable server name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Server implementation version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Visibility profile applied independently to every request.
    pub fn profile(&self) -> &McpProfile {
        &self.profile
    }
    /// Native cards projected through the canonical MCP surface path.
    pub fn native_cards(&self) -> &[McpNativeCard] {
        &self.native_cards
    }
    /// Capabilities explicitly granted by the service owner.
    pub fn granted_capabilities(&self) -> &[CapabilityName] {
        &self.granted_capabilities
    }
}

/// Extensions negotiated by an outer protocol adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NegotiatedExtensions(BTreeSet<String>);

impl NegotiatedExtensions {
    /// Creates an empty extension set.
    pub fn none() -> Self {
        Self::default()
    }
    /// Adds one negotiated extension identifier.
    pub fn with(mut self, extension: impl Into<String>) -> Self {
        self.0.insert(extension.into());
        self
    }
    /// Reports whether an extension was negotiated.
    pub fn contains(&self, extension: &str) -> bool {
        self.0.contains(extension)
    }
}

/// Authenticated caller identity supplied by the hosting boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    subject: String,
    claims: BTreeMap<String, String>,
}

impl Principal {
    /// Creates a principal for `subject`.
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            claims: BTreeMap::new(),
        }
    }
    /// Adds a non-secret identity claim.
    pub fn with_claim(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.claims.insert(name.into(), value.into());
        self
    }
    /// Stable subject identifier.
    pub fn subject(&self) -> &str {
        &self.subject
    }
    /// Returns one non-secret identity claim.
    pub fn claim(&self, name: &str) -> Option<&str> {
        self.claims.get(name).map(String::as_str)
    }
}

/// Cache behavior chosen by the request boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CachePolicy {
    /// Do not read or write a response cache.
    #[default]
    Bypass,
    /// A deterministic cache may be read but not populated.
    ReadOnly,
    /// A deterministic cache may be read and populated.
    ReadWrite,
}

/// Complete immutable facts for one decoded request.
#[derive(Clone, Debug)]
pub struct RequestContext {
    request_id: String,
    protocol_version: String,
    extensions: NegotiatedExtensions,
    principal: Principal,
    cache_policy: CachePolicy,
}

impl RequestContext {
    /// Creates a complete request context.
    pub fn new(
        request_id: impl Into<String>,
        protocol_version: impl Into<String>,
        extensions: NegotiatedExtensions,
        principal: Principal,
        cache_policy: CachePolicy,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            protocol_version: protocol_version.into(),
            extensions,
            principal,
            cache_policy,
        }
    }
    /// Request correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    /// Negotiated protocol version.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }
    /// Negotiated extensions.
    pub fn extensions(&self) -> &NegotiatedExtensions {
        &self.extensions
    }
    /// Authenticated caller.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
    /// Selected cache behavior.
    pub fn cache_policy(&self) -> CachePolicy {
        self.cache_policy
    }
}

/// Ordered response envelopes emitted for one service request.
#[derive(Debug)]
pub struct ServiceResponseStream(std::vec::IntoIter<McpEnvelope>);

impl Iterator for ServiceResponseStream {
    type Item = McpEnvelope;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl ExactSizeIterator for ServiceResponseStream {}

/// Immutable, concurrency-safe MCP application service.
pub struct McpService {
    description: ServerDescription,
}

impl McpService {
    /// Creates a service from its immutable description.
    pub fn new(description: ServerDescription) -> Self {
        Self { description }
    }
    /// Returns the immutable service description.
    pub fn description(&self) -> &ServerDescription {
        &self.description
    }

    /// Handles one already-decoded request using a fresh caller-owned context.
    pub fn handle(
        &self,
        cx: &mut Cx,
        context: &RequestContext,
        request: McpRequest,
    ) -> Result<ServiceResponseStream> {
        if matches!(
            request.method.as_str(),
            "initialize" | "initialized" | "notifications/initialized" | "shutdown"
        ) {
            let error = McpEnvelope::Error(McpErrorEnvelope {
                id: request.id,
                error: McpError {
                    code: METHOD_NOT_FOUND,
                    message: "connection lifecycle method belongs to a protocol adapter".to_owned(),
                    data: Expr::String("use sim-lib-mcp-legacy for initialize-era MCP".to_owned()),
                },
            });
            return Ok(ServiceResponseStream(vec![error].into_iter()));
        }
        let mut session = McpSession::new(context.request_id(), self.description.profile.clone())
            .with_native_cards(self.description.native_cards.clone());
        session.protocol_version = context.protocol_version.clone();
        for capability in &self.description.granted_capabilities {
            session = session.with_granted_capability(capability.clone());
        }
        let replies = McpRouter::new(session).handle_many(cx, McpEnvelope::Request(request))?;
        Ok(ServiceResponseStream(replies.into_iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_codec_mcp::{McpEnvelope, McpRequest};
    use sim_kernel::{Cx, Expr};

    #[test]
    fn service_is_stateless_and_rejects_connection_lifecycle() {
        let service = McpService::new(ServerDescription::new("sim", "1", McpProfile::all()));
        let context = RequestContext::new(
            "r1",
            "2025-03-26",
            NegotiatedExtensions::none(),
            Principal::new("fixture"),
            CachePolicy::Bypass,
        );
        let request = McpRequest {
            id: Expr::String("1".into()),
            method: "initialize".into(),
            params: Expr::Nil,
        };
        assert!(matches!(
            service
                .handle(&mut Cx::new(), &context, request)
                .unwrap()
                .next(),
            Some(McpEnvelope::Error(_))
        ));
    }
}
