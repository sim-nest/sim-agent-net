/// Result type for protocol validation.
pub type Result<T> = std::result::Result<T, OAuthError>;

/// Sanitized OAuth failure. It never contains token or verifier material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuthError(pub &'static str);
impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for OAuthError {}

/// A validated absolute HTTPS URL (loopback HTTP may be explicitly admitted).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SecureUrl(String);
impl SecureUrl {
    /// Validates a URL for metadata or endpoint use, rejecting credentials and fragments.
    pub fn parse(value: &str, allow_loopback_http: bool) -> Result<Self> {
        let parsed = Url::parse(value).map_err(|_| OAuthError("invalid absolute URL"))?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(OAuthError("URL credentials and fragments are forbidden"));
        }
        let loopback = parsed
            .host_str()
            .is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h == "::1");
        if parsed.scheme() != "https"
            && !(allow_loopback_http && parsed.scheme() == "http" && loopback)
        {
            return Err(OAuthError("OAuth URL must use HTTPS"));
        }
        if parsed.host_str().is_none() {
            return Err(OAuthError("OAuth URL requires an origin"));
        }
        Ok(Self(parsed.to_string()))
    }
    /// Returns the serialized validated URL.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Returns the exact scheme/host/port origin.
    pub fn origin(&self) -> Result<String> {
        let u = Url::parse(&self.0).map_err(|_| OAuthError("invalid stored URL"))?;
        Ok(u.origin().ascii_serialization())
    }
}

/// Deterministically ordered, non-empty OAuth scopes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScopeSet(BTreeSet<String>);
impl ScopeSet {
    /// Parses an ASCII space-delimited scope value with bounded tokens.
    pub fn parse(value: &str) -> Result<Self> {
        let mut out = BTreeSet::new();
        for item in value.split_ascii_whitespace() {
            if item.len() > 128
                || item.is_empty()
                || !item
                    .bytes()
                    .all(|b| (0x21..=0x7e).contains(&b) && b != b'"' && b != b'\\')
            {
                return Err(OAuthError("invalid OAuth scope"));
            }
            out.insert(item.to_owned());
        }
        Ok(Self(out))
    }
    /// Returns true when every required scope is present.
    pub fn contains_all(&self, required: &Self) -> bool {
        required.0.is_subset(&self.0)
    }
    /// Produces canonical scope text.
    pub fn canonical(&self) -> String {
        self.0.iter().cloned().collect::<Vec<_>>().join(" ")
    }
}

/// Bounded extension values retained from discovery.
pub type Extensions = BTreeMap<String, String>;

/// RFC 9728 protected-resource metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedResourceMetadata {
    /// Exact resource identifier.
    pub resource: SecureUrl,
    /// Ordered authorization-server issuers.
    pub authorization_servers: Vec<SecureUrl>,
    /// Scopes understood by this resource.
    pub scopes_supported: ScopeSet,
    /// Bounded extension fields.
    pub extensions: Extensions,
}

/// Authorization-server/OpenID discovery metadata used by the client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationServerMetadata {
    /// Exact issuer; discovery responses must equal it byte-for-byte after URL validation.
    pub issuer: SecureUrl,
    /// Authorization endpoint.
    pub authorization_endpoint: SecureUrl,
    /// Token endpoint.
    pub token_endpoint: SecureUrl,
    /// Optional registration endpoint.
    pub registration_endpoint: Option<SecureUrl>,
    /// Advertised PKCE methods.
    pub code_challenge_methods_supported: BTreeSet<String>,
    /// Bounded extensions.
    pub extensions: Extensions,
}
impl AuthorizationServerMetadata {
    /// Enforces issuer equality, endpoint policy, and mandatory S256.
    pub fn validate(
        &self,
        expected_issuer: &SecureUrl,
        allow_cross_origin_endpoints: bool,
    ) -> Result<()> {
        if self.issuer != *expected_issuer {
            return Err(OAuthError("authorization issuer mismatch"));
        }
        if !self.code_challenge_methods_supported.contains("S256") {
            return Err(OAuthError(
                "authorization server does not support PKCE S256",
            ));
        }
        if !allow_cross_origin_endpoints {
            let origin = self.issuer.origin()?;
            if self.authorization_endpoint.origin()? != origin
                || self.token_endpoint.origin()? != origin
            {
                return Err(OAuthError("cross-origin authorization endpoint forbidden"));
            }
        }
        Ok(())
    }
}

/// Parsed Bearer challenge parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BearerChallenge {
    /// Protected-resource metadata URL.
    pub resource_metadata: Option<SecureUrl>,
    /// Requested scope step-up.
    pub scope: ScopeSet,
    /// Standard error code.
    pub error: Option<String>,
}
impl BearerChallenge {
    /// Parses a bounded, single Bearer challenge and rejects control characters/duplicates.
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(OAuthError("malformed authentication challenge"));
        }
        let rest = value
            .strip_prefix("Bearer ")
            .ok_or(OAuthError("unsupported authentication scheme"))?;
        let mut fields = BTreeMap::new();
        for part in rest.split(',') {
            let (k, raw) = part
                .trim()
                .split_once('=')
                .ok_or(OAuthError("malformed authentication challenge"))?;
            let v = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(OAuthError("challenge values must be quoted"))?;
            if fields.insert(k, v).is_some() {
                return Err(OAuthError("duplicate challenge parameter"));
            }
        }
        Ok(Self {
            resource_metadata: fields
                .get("resource_metadata")
                .map(|v| SecureUrl::parse(v, false))
                .transpose()?,
            scope: ScopeSet::parse(fields.get("scope").copied().unwrap_or(""))?,
            error: fields.get("error").map(|v| (*v).to_owned()),
        })
    }
}

/// Opaque secret whose printable faces are always redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);
impl Secret {
    /// Creates a non-empty single-line secret.
    pub fn new(v: impl Into<String>) -> Result<Self> {
        let v = v.into();
        if v.is_empty() || v.chars().any(char::is_control) {
            Err(OAuthError("invalid secret material"))
        } else {
            Ok(Self(v))
        }
    }
    /// Exposes the secret only at its immediate cryptographic/transport boundary.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<secret>")
    }
}
impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<secret>")
    }
}

/// Injected cryptographically secure random source.
pub trait SecureRandom {
    /// Produces exactly `bytes` unpredictable bytes.
    fn random(&mut self, bytes: usize) -> Result<Vec<u8>>;
}
/// Explicit browser interface.
pub trait Browser {
    /// Opens the authorization URL after consent.
    fn open(&mut self, url: &SecureUrl) -> Result<()>;
}
/// Explicit user-consent interface.
pub trait Consent {
    /// Confirms issuer/resource/scopes before browser launch.
    fn approve(
        &mut self,
        issuer: &SecureUrl,
        resource: &SecureUrl,
        scopes: &ScopeSet,
    ) -> Result<()>;
}
/// Explicit token persistence interface.
pub trait TokenStore {
    /// Stores a token under an opaque principal/issuer handle.
    fn store(&mut self, handle: &str, token: Secret) -> Result<()>;
}

/// Client registration policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Registration {
    /// Preconfigured client identifier.
    Configured {
        /// Client identifier.
        client_id: String,
    },
    /// Client-id metadata document URL.
    ClientMetadataDocument {
        /// HTTPS metadata URL.
        url: SecureUrl,
    },
    /// Optional legacy dynamic registration, requiring an exact allowlist entry.
    DeprecatedDynamic {
        /// Allowlisted registration endpoint.
        endpoint: SecureUrl,
    },
}
impl Registration {
    /// Validates dynamic registration against explicit policy.
    pub fn validate(&self, allow_dynamic: bool, allowed: &[SecureUrl]) -> Result<()> {
        if let Self::DeprecatedDynamic { endpoint } = self
            && (!allow_dynamic || !allowed.contains(endpoint))
        {
            return Err(OAuthError("dynamic registration is disabled by policy"));
        }
        Ok(())
    }
}
