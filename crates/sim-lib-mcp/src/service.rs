//! Stateless MCP service boundary.
//!
//! Connection negotiation belongs to adapters. The service owns only an
//! immutable description and receives every request fact explicitly.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
};

use sim_codec_mcp::{
    METHOD_NOT_FOUND, McpEnvelope, McpError, McpErrorEnvelope, McpRequest, McpResponse,
};
use sim_codec_mcp::{Method, ResultType, method_registry};
use sim_kernel::{CapabilityName, CapabilitySet, Cx, Expr, HandleSeed, Result, diminish};

use crate::{McpNativeCard, McpProfile, McpRouter, McpSession};

/// Immutable description of one MCP service.
#[derive(Clone)]
pub struct ServerDescription {
    name: String,
    version: String,
    profile: McpProfile,
    native_cards: Vec<McpNativeCard>,
    granted_capabilities: Vec<CapabilityName>,
    supported_versions: Vec<String>,
    extensions: BTreeSet<String>,
    discovery_ttl_seconds: u64,
    cache_scope: String,
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
            supported_versions: vec!["2026-07-28".to_owned()],
            extensions: BTreeSet::new(),
            discovery_ttl_seconds: 60,
            cache_scope: "public".to_owned(),
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
    /// Replaces the exact supported protocol revisions in preference order.
    pub fn with_supported_versions(mut self, versions: Vec<String>) -> Self {
        self.supported_versions = versions;
        self
    }
    /// Adds one installed, discoverable extension identifier.
    pub fn with_extension(mut self, extension: impl Into<String>) -> Self {
        self.extensions.insert(extension.into());
        self
    }
    /// Sets discovery freshness and cache scope.
    pub fn with_discovery_cache(mut self, ttl_seconds: u64, scope: impl Into<String>) -> Self {
        self.discovery_ttl_seconds = ttl_seconds;
        self.cache_scope = scope.into();
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
    principal_grants: CapabilitySet,
    admitted_needs: CapabilitySet,
    input_capabilities: BTreeSet<String>,
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
            principal_grants: CapabilitySet::new(),
            admitted_needs: CapabilitySet::new(),
            input_capabilities: BTreeSet::new(),
        }
    }
    /// Replaces the authenticated principal's grants.
    pub fn with_principal_grants(mut self, grants: CapabilitySet) -> Self {
        self.principal_grants = grants;
        self
    }
    /// Replaces the operation capabilities admitted by host policy.
    pub fn with_admitted_needs(mut self, needs: CapabilitySet) -> Self {
        self.admitted_needs = needs;
        self
    }
    /// Declares one caller input capability for this request.
    pub fn with_input_capability(mut self, capability: impl Into<String>) -> Self {
        self.input_capabilities.insert(capability.into());
        self
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
    /// Authenticated grants before operation diminution.
    pub fn principal_grants(&self) -> &CapabilitySet {
        &self.principal_grants
    }
    /// Host-admitted needs for the addressed operation.
    pub fn admitted_needs(&self) -> &CapabilitySet {
        &self.admitted_needs
    }
    /// Input capabilities declared by this request.
    pub fn input_capabilities(&self) -> &BTreeSet<String> {
        &self.input_capabilities
    }
}

/// Forks isolated request contexts from an explicit host seed.
///
/// The factory owns only a handle namespace counter. The seed's behavior
/// catalog is copied by [`Cx::fork_from_seed`], while request stores, ledgers,
/// diagnostics, continuations, traces, cancellation, and grant seats are new.
#[derive(Debug, Default)]
pub struct RequestCxFactory {
    next_handle_seed: AtomicU64,
}

impl RequestCxFactory {
    /// Starts a factory at a host-selected non-secret handle namespace.
    pub const fn new(first_handle_seed: u64) -> Self {
        Self {
            next_handle_seed: AtomicU64::new(first_handle_seed),
        }
    }

    /// Runs one action in a fresh diminished context.
    pub fn run<T>(
        &self,
        host_seed: &Cx,
        request: &RequestContext,
        action: impl FnOnce(&mut Cx) -> Result<T>,
    ) -> Result<T> {
        let mut fresh = host_seed.fork_from_seed(HandleSeed::new(
            self.next_handle_seed.fetch_add(1, Ordering::Relaxed),
        ));
        let admitted = diminish(request.principal_grants(), request.admitted_needs());
        fresh.with_capabilities(admitted, action)
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
    request_contexts: RequestCxFactory,
    providers: std::sync::Arc<crate::McpProviders>,
}

impl McpService {
    /// Creates a service from its immutable description.
    pub fn new(description: ServerDescription) -> Self {
        Self {
            description,
            request_contexts: RequestCxFactory::default(),
            providers: std::sync::Arc::new(crate::McpProviders::default()),
        }
    }
    /// Installs the explicit construction-time shared provider catalog.
    pub fn with_providers(mut self, providers: crate::McpProviders) -> Self {
        self.providers = std::sync::Arc::new(providers);
        self
    }
    /// Returns the immutable service description.
    pub fn description(&self) -> &ServerDescription {
        &self.description
    }

    /// Handles one already-decoded request in a fresh diminished context forked
    /// from the explicit host seed.
    pub fn handle(
        &self,
        host_seed: &mut Cx,
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
        if request.method == "server/discover" {
            return Ok(ServiceResponseStream(
                vec![McpEnvelope::Response(McpResponse {
                    id: request.id,
                    result: self.discover(),
                })]
                .into_iter(),
            ));
        }
        self.request_contexts.run(host_seed, context, |cx| {
            let mut session =
                McpSession::new(context.request_id(), self.description.profile.clone())
                    .with_native_cards(self.description.native_cards.clone());
            session.protocol_version = context.protocol_version.clone();
            for capability in context
                .principal_grants()
                .intersect(context.admitted_needs())
                .iter()
            {
                session = session.with_granted_capability(capability.clone());
            }
            let replies = McpRouter::new(session).handle_many(cx, McpEnvelope::Request(request))?;
            Ok(ServiceResponseStream(replies.into_iter()))
        })
    }

    /// Builds mandatory discovery solely from the immutable description and
    /// explicitly injected provider names.
    pub fn discover(&self) -> Expr {
        let strings =
            |values: Vec<String>| Expr::Vector(values.into_iter().map(Expr::String).collect());
        let provider_names = self
            .providers
            .durable
            .keys()
            .chain(self.providers.events.keys())
            .cloned()
            .collect();
        Expr::Map(vec![
            (
                Expr::String("resultType".into()),
                Expr::String("complete".into()),
            ),
            (
                Expr::String("supportedVersions".into()),
                strings(self.description.supported_versions.clone()),
            ),
            (
                Expr::String("extensions".into()),
                strings(self.description.extensions.iter().cloned().collect()),
            ),
            (Expr::String("providers".into()), strings(provider_names)),
            (
                Expr::String("serverInfo".into()),
                Expr::Map(vec![
                    (
                        Expr::String("name".into()),
                        Expr::String(self.description.name.clone()),
                    ),
                    (
                        Expr::String("version".into()),
                        Expr::String(self.description.version.clone()),
                    ),
                ]),
            ),
            (
                Expr::String("ttl".into()),
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: sim_kernel::Symbol::qualified("numbers", "u64"),
                    canonical: self.description.discovery_ttl_seconds.to_string(),
                }),
            ),
            (
                Expr::String("cacheScope".into()),
                Expr::String(self.description.cache_scope.clone()),
            ),
        ])
    }

    /// Returns a truthful cache hint only for codec-declared eligible methods
    /// and complete results.
    pub fn cache_hint(
        &self,
        method: Method,
        result_type: &ResultType,
        context: &RequestContext,
    ) -> Option<sim_codec_mcp::CacheHint> {
        (method_registry(method).cache_eligible
            && matches!(result_type, ResultType::Complete)
            && context.cache_policy() != CachePolicy::Bypass)
            .then_some(sim_codec_mcp::CacheHint {
                cacheable: true,
                max_age: Some(60),
            })
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
                .handle(&mut sim_kernel::testing::bare_cx(), &context, request)
                .unwrap()
                .next(),
            Some(McpEnvelope::Error(_))
        ));
    }

    #[test]
    fn request_factory_intersects_authority_and_discards_request_state() {
        let allowed = CapabilityName::new("mcp.allowed");
        let denied = CapabilityName::new("mcp.denied");
        let context = RequestContext::new(
            "r1",
            "2026-07-28",
            NegotiatedExtensions::none(),
            Principal::new("alice"),
            CachePolicy::Bypass,
        )
        .with_principal_grants(
            CapabilitySet::new()
                .grant(allowed.clone())
                .grant(denied.clone()),
        )
        .with_admitted_needs(CapabilitySet::new().grant(allowed.clone()));
        let mut seed = sim_kernel::testing::bare_cx();
        seed.push_info("host diagnostic must not leak");
        let factory = RequestCxFactory::new(100);
        let first = factory
            .run(&seed, &context, |cx| {
                assert!(cx.capabilities().contains(&allowed));
                assert!(!cx.capabilities().contains(&denied));
                assert!(cx.diagnostics().is_empty());
                cx.push_info("request-private");
                Ok(cx.fresh_handle())
            })
            .unwrap();
        let second = factory
            .run(&seed, &context, |cx| {
                assert!(cx.diagnostics().is_empty());
                Ok(cx.fresh_handle())
            })
            .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn cache_hints_follow_codec_registry_and_complete_status() {
        let service = McpService::new(ServerDescription::new("sim", "1", McpProfile::all()));
        let context = RequestContext::new(
            "r",
            "2026-07-28",
            NegotiatedExtensions::none(),
            Principal::new("alice"),
            CachePolicy::ReadWrite,
        );
        assert!(
            service
                .cache_hint(Method::ToolsList, &ResultType::Complete, &context)
                .is_some()
        );
        assert!(
            service
                .cache_hint(Method::ToolsCall, &ResultType::Complete, &context)
                .is_none()
        );
        assert!(
            service
                .cache_hint(Method::ToolsList, &ResultType::InputRequired, &context)
                .is_none()
        );
    }

    #[test]
    fn randomized_request_order_cannot_change_authority_or_private_state() {
        let factory = RequestCxFactory::new(500);
        let seed = sim_kernel::testing::bare_cx();
        let read = CapabilityName::new("mcp.read");
        let write = CapabilityName::new("mcp.write");
        let contexts = [
            RequestContext::new(
                "a",
                "2026-07-28",
                NegotiatedExtensions::none(),
                Principal::new("alice"),
                CachePolicy::ReadWrite,
            )
            .with_principal_grants(CapabilitySet::new().grant(read.clone()))
            .with_admitted_needs(
                CapabilitySet::new()
                    .grant(read.clone())
                    .grant(write.clone()),
            ),
            RequestContext::new(
                "b",
                "2025-03-26",
                NegotiatedExtensions::none().with("example/x"),
                Principal::new("bob"),
                CachePolicy::Bypass,
            )
            .with_principal_grants(CapabilitySet::new().grant(write.clone()))
            .with_admitted_needs(CapabilitySet::new().grant(write.clone())),
        ];
        let observe = |context: &RequestContext| {
            factory
                .run(&seed, context, |cx| {
                    assert!(cx.diagnostics().is_empty());
                    Ok(cx
                        .capabilities()
                        .iter()
                        .map(|cap| cap.as_str().to_owned())
                        .collect::<Vec<_>>())
                })
                .unwrap()
        };
        let expected = [observe(&contexts[0]), observe(&contexts[1])];
        let mut state = 0x5eed_u64;
        for _ in 0..128 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let order = if state & 1 == 0 { [0, 1] } else { [1, 0] };
            for index in order {
                assert_eq!(observe(&contexts[index]), expected[index]);
            }
        }
    }
}
