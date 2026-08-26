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
        if !matches!(origins, OriginPolicy::LoopbackOnly) {
            return Err(Error::Eval(
                "ServerPolicy::new is explicit anonymous loopback development mode; use remote for other binds".into(),
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

    /// Constructs policy for a non-loopback endpoint, requiring exact origins and a configured verifier.
    pub fn remote(
        endpoint: impl Into<String>,
        origins: Vec<String>,
        max_body_bytes: usize,
        verifier_configured: bool,
    ) -> Result<Self> {
        if origins.is_empty() || !verifier_configured {
            return Err(Error::Eval(
                "remote MCP HTTP requires exact Origin policy and a configured verifier".into(),
            ));
        }
        let endpoint = endpoint.into();
        if !endpoint.starts_with('/') || max_body_bytes == 0 {
            return Err(Error::Eval(
                "MCP HTTP endpoint must be absolute and body cap non-zero".into(),
            ));
        }
        Ok(Self {
            endpoint,
            legacy_endpoint: None,
            origins: OriginPolicy::Exact(origins),
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
    /// Host-authorized grants derived solely from verified scopes and policy.
    pub grants: CapabilitySet,
}

/// Authentication refusal rendered without secret material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthRejection {
    /// HTTP status, exactly 401 or 403.
    pub status: u16,
    /// RFC 6750 Bearer challenge.
    pub challenge: String,
}

/// Host policy that authenticates a request without granting ambient authority.
pub trait IdentityProvider: Send + Sync {
    /// Resolves one request identity from already-parsed request facts.
    fn identify(&self, head: &RequestHead) -> std::result::Result<RequestIdentity, AuthRejection>;
}

/// Exact scope-to-capability diminution policy.
#[derive(Clone, Debug, Default)]
pub struct ScopeGrantPolicy {
    mappings: Vec<(String, CapabilityName)>,
}

/// Binding of OAuth authority to an existing Provider_4-compatible seat.
///
/// This adapter records only the redaction-safe seat card and uses the existing
/// provider authentication vocabulary; OAuth does not create an account model.
#[derive(Clone, Debug)]
pub struct ProviderOAuthBinding {
    seat: ProviderSeatCard,
}
impl ProviderOAuthBinding {
    /// Binds one existing provider seat after confirming OAuth is an advertised operation.
    pub fn new(seat: ProviderSeatCard) -> Result<Self> {
        let supports_oauth = seat.auth_metadata()?.is_some();
        if !supports_oauth {
            return Err(Error::Eval(
                "provider seat has no typed authentication contract".into(),
            ));
        }
        Ok(Self { seat })
    }
    /// Existing provider authentication operation used for this binding.
    pub fn method(&self) -> AuthMethod {
        AuthMethod::OauthBrowser
    }
    /// Redaction-safe seat selected by Provider_4 composition.
    pub fn seat(&self) -> &ProviderSeatCard {
        &self.seat
    }
}
impl ScopeGrantPolicy {
    /// Adds one exact scope mapping; self-reported client and network facts are ignored.
    pub fn map(mut self, scope: impl Into<String>, capability: CapabilityName) -> Self {
        self.mappings.push((scope.into(), capability));
        self
    }
    fn grants(&self, scopes: &ScopeSet) -> CapabilitySet {
        self.mappings
            .iter()
            .filter(|(scope, _)| ScopeSet::parse(scope).is_ok_and(|one| scopes.contains_all(&one)))
            .fold(CapabilitySet::new(), |set, (_, cap)| set.grant(cap.clone()))
    }
}

/// OAuth bearer authentication performed before `RequestContext` construction.
pub struct OAuthIdentityProvider<V, N> {
    verifier: V,
    now: N,
    issuer: SecureUrl,
    resource: SecureUrl,
    resource_metadata: SecureUrl,
    required_scopes: ScopeSet,
    grants: ScopeGrantPolicy,
}
impl<V, N> OAuthIdentityProvider<V, N> {
    /// Binds exact authority, resource, scope, metadata, clock, and grant policy.
    pub fn new(
        verifier: V,
        now: N,
        issuer: SecureUrl,
        resource: SecureUrl,
        resource_metadata: SecureUrl,
        required_scopes: ScopeSet,
        grants: ScopeGrantPolicy,
    ) -> Self {
        Self {
            verifier,
            now,
            issuer,
            resource,
            resource_metadata,
            required_scopes,
            grants,
        }
    }
    fn challenge(&self, status: u16) -> AuthRejection {
        let error = if status == 403 {
            "insufficient_scope"
        } else {
            "invalid_token"
        };
        AuthRejection {
            status,
            challenge: format!(
                "Bearer resource_metadata=\"{}\", error=\"{}\", scope=\"{}\"",
                self.resource_metadata.as_str(),
                error,
                self.required_scopes.canonical()
            ),
        }
    }
}
impl<V: AccessTokenVerifier, N: Fn() -> u64 + Send + Sync> IdentityProvider
    for OAuthIdentityProvider<V, N>
{
    fn identify(&self, head: &RequestHead) -> std::result::Result<RequestIdentity, AuthRejection> {
        let values = header_values(head, "authorization");
        if values.len() != 1 {
            return Err(self.challenge(401));
        }
        let material = values[0]
            .strip_prefix("Bearer ")
            .ok_or_else(|| self.challenge(401))?;
        let token = Secret::new(material).map_err(|_| self.challenge(401))?;
        let verified = self
            .verifier
            .verify(
                &token,
                &self.issuer,
                &self.resource,
                &self.required_scopes,
                (self.now)(),
            )
            .map_err(|e| {
                self.challenge(if e.0 == "insufficient token scope" {
                    403
                } else {
                    401
                })
            })?;
        let grants = self.grants.grants(verified.scopes());
        let principal = Principal::new(verified.subject())
            .with_claim("issuer", verified.issuer().as_str())
            .with_claim("resource", verified.resource().as_str())
            .with_claim("scope", verified.scopes().canonical());
        Ok(RequestIdentity { principal, grants })
    }
}

/// RFC 9728 protected-resource metadata endpoint over the shared raw HTTP seam.
pub struct ProtectedResourceHandler<K> {
    endpoint: String,
    resource: SecureUrl,
    authorization_servers: Vec<SecureUrl>,
    scopes: ScopeSet,
    clock: K,
}
impl<K> ProtectedResourceHandler<K> {
    /// Configures one exact well-known endpoint and its public authority facts.
    pub fn new(
        endpoint: impl Into<String>,
        resource: SecureUrl,
        authorization_servers: Vec<SecureUrl>,
        scopes: ScopeSet,
        clock: K,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        if !endpoint.starts_with("/.well-known/") || authorization_servers.is_empty() {
            return Err(Error::Eval(
                "protected-resource metadata requires a well-known endpoint and authority".into(),
            ));
        }
        Ok(Self {
            endpoint,
            resource,
            authorization_servers,
            scopes,
            clock,
        })
    }
}
impl<K: HttpClock> RawHandler for ProtectedResourceHandler<K> {
    fn handle(
        &self,
        head: &RequestHead,
        _body: &mut dyn BodyReader,
        response: &mut dyn ResponseWriter,
        scope: &RequestScope,
    ) -> Result<()> {
        if target_path(&head.target) != self.endpoint {
            return empty(
                response,
                scope,
                404,
                common_headers(&self.clock.http_date()),
            );
        }
        if head.method != "GET" {
            let mut headers = common_headers(&self.clock.http_date());
            headers.push(("Allow".into(), "GET".into()));
            return empty(response, scope, 405, headers);
        }
        let bytes=serde_json::to_vec(&serde_json::json!({"resource":self.resource.as_str(),"authorization_servers":self.authorization_servers.iter().map(SecureUrl::as_str).collect::<Vec<_>>(),"scopes_supported":self.scopes.canonical().split_ascii_whitespace().collect::<Vec<_>>()})).map_err(|_|Error::Eval("protected-resource metadata encoding failed".into()))?;
        let mut headers = common_headers(&self.clock.http_date());
        headers.push(("Content-Type".into(), "application/json".into()));
        response
            .write_head(
                ResponseHead {
                    status: 200,
                    headers,
                },
                scope,
            )
            .map_err(Error::host_io)?;
        response
            .write_chunk(&bytes, scope)
            .map_err(Error::host_io)?;
        response.finish(&[], scope).map_err(Error::host_io)
    }
}
