//! Effect-free OAuth 2.1 resource/client state and authority facts.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use url::Url;

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

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
        if let Self::DeprecatedDynamic { endpoint } = self {
            if !allow_dynamic || !allowed.contains(endpoint) {
                return Err(OAuthError("dynamic registration is disabled by policy"));
            }
        }
        Ok(())
    }
}

/// Pending authorization-code transaction.
#[derive(Clone, Debug)]
pub struct AuthorizationCodeFlow {
    issuer: SecureUrl,
    resource: SecureUrl,
    scopes: ScopeSet,
    state: Secret,
    verifier: Secret,
    redirect_uri: SecureUrl,
}
/// Values safe to send to an authorization endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    /// Exact query pairs.
    pub parameters: Vec<(String, String)>,
}
impl AuthorizationCodeFlow {
    /// Begins a PKCE S256 flow using injected secure randomness.
    pub fn begin(
        random: &mut dyn SecureRandom,
        issuer: SecureUrl,
        resource: SecureUrl,
        scopes: ScopeSet,
        redirect_uri: SecureUrl,
        client_id: &str,
    ) -> Result<(Self, AuthorizationRequest)> {
        if client_id.is_empty() {
            return Err(OAuthError("client identifier is empty"));
        }
        let state = Secret::new(URL_SAFE_NO_PAD.encode(random.random(32)?))?;
        let verifier = Secret::new(URL_SAFE_NO_PAD.encode(random.random(32)?))?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes()));
        let request = AuthorizationRequest {
            parameters: vec![
                ("response_type".into(), "code".into()),
                ("client_id".into(), client_id.into()),
                ("redirect_uri".into(), redirect_uri.as_str().into()),
                ("scope".into(), scopes.canonical()),
                ("resource".into(), resource.as_str().into()),
                ("state".into(), state.expose().into()),
                ("code_challenge".into(), challenge),
                ("code_challenge_method".into(), "S256".into()),
            ],
        };
        Ok((
            Self {
                issuer,
                resource,
                scopes,
                state,
                verifier,
                redirect_uri,
            },
            request,
        ))
    }
    /// Validates the redirect and returns a resource-bound token request.
    pub fn finish(self, response: AuthorizationResponse, client_id: &str) -> Result<TokenRequest> {
        if response.state != self.state.expose() {
            return Err(OAuthError("authorization state mismatch"));
        }
        if response.issuer.as_ref() != Some(&self.issuer) {
            return Err(OAuthError("authorization issuer response mismatch"));
        }
        if response.code.is_empty() {
            return Err(OAuthError("authorization code is empty"));
        }
        Ok(TokenRequest {
            code: Secret::new(response.code)?,
            verifier: self.verifier,
            client_id: client_id.into(),
            redirect_uri: self.redirect_uri,
            resource: self.resource,
            scopes: self.scopes,
        })
    }
}
/// Authorization endpoint redirect facts.
pub struct AuthorizationResponse {
    /// Code.
    pub code: String,
    /// State.
    pub state: String,
    /// RFC 9207 issuer response.
    pub issuer: Option<SecureUrl>,
}
/// Resource-bound authorization-code token exchange.
#[derive(Debug)]
pub struct TokenRequest {
    /// Opaque code.
    pub code: Secret,
    /// PKCE verifier.
    pub verifier: Secret,
    /// Client id.
    pub client_id: String,
    /// Exact redirect URI.
    pub redirect_uri: SecureUrl,
    /// Exact resource indicator.
    pub resource: SecureUrl,
    /// Least requested scopes.
    pub scopes: ScopeSet,
}

/// Immutable authenticated identity facts returned by token verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPrincipal {
    issuer: SecureUrl,
    subject: String,
    audience: BTreeSet<String>,
    resource: SecureUrl,
    scopes: ScopeSet,
    expires_at: u64,
    token_id: Option<String>,
}
impl VerifiedPrincipal {
    /// Constructs checked identity facts; token bytes are intentionally absent.
    pub fn new(
        issuer: SecureUrl,
        subject: String,
        audience: BTreeSet<String>,
        resource: SecureUrl,
        scopes: ScopeSet,
        expires_at: u64,
        token_id: Option<String>,
    ) -> Result<Self> {
        if subject.is_empty() || audience.is_empty() {
            return Err(OAuthError("incomplete verified principal"));
        }
        Ok(Self {
            issuer,
            subject,
            audience,
            resource,
            scopes,
            expires_at,
            token_id,
        })
    }
    /// Issuer.
    pub fn issuer(&self) -> &SecureUrl {
        &self.issuer
    }
    /// Subject.
    pub fn subject(&self) -> &str {
        &self.subject
    }
    /// Audiences.
    pub fn audience(&self) -> &BTreeSet<String> {
        &self.audience
    }
    /// Resource.
    pub fn resource(&self) -> &SecureUrl {
        &self.resource
    }
    /// Scopes.
    pub fn scopes(&self) -> &ScopeSet {
        &self.scopes
    }
    /// Expiry epoch seconds.
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    /// Token identifier.
    pub fn token_id(&self) -> Option<&str> {
        self.token_id.as_deref()
    }
}
/// Injected access-token verification boundary (local JOSE or remote introspection).
pub trait AccessTokenVerifier: Send + Sync {
    /// Verifies without retaining or printing token bytes.
    fn verify(
        &self,
        token: &Secret,
        expected_issuer: &SecureUrl,
        expected_resource: &SecureUrl,
        required_scopes: &ScopeSet,
        now: u64,
    ) -> Result<VerifiedPrincipal>;
}

/// Caps a discovery document before parsing.
pub fn validate_document_size(bytes: &[u8], maximum: usize) -> Result<()> {
    if maximum == 0 || bytes.len() > maximum {
        Err(OAuthError("OAuth discovery document exceeds policy cap"))
    } else {
        Ok(())
    }
}
/// Validates an exact redirect URI.
pub fn validate_redirect(expected: &SecureUrl, actual: &str) -> Result<()> {
    if expected.as_str() != actual {
        Err(OAuthError("redirect URI mismatch"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct R(u8);
    impl SecureRandom for R {
        fn random(&mut self, n: usize) -> Result<Vec<u8>> {
            self.0 += 1;
            Ok(vec![self.0; n])
        }
    }
    #[test]
    fn pkce_state_issuer_and_resource_are_bound() {
        let i = SecureUrl::parse("https://as.example/", false).unwrap();
        let r = SecureUrl::parse("https://mcp.example/", false).unwrap();
        let red = SecureUrl::parse("https://client.example/cb", false).unwrap();
        let (flow, req) = AuthorizationCodeFlow::begin(
            &mut R(0),
            i.clone(),
            r.clone(),
            ScopeSet::parse("read").unwrap(),
            red,
            "client",
        )
        .unwrap();
        assert!(
            req.parameters
                .contains(&("resource".into(), r.as_str().into()))
        );
        let state = req
            .parameters
            .iter()
            .find(|p| p.0 == "state")
            .unwrap()
            .1
            .clone();
        assert!(
            flow.finish(
                AuthorizationResponse {
                    code: "c".into(),
                    state,
                    issuer: Some(i)
                },
                "client"
            )
            .is_ok()
        );
    }
    #[test]
    fn secrets_and_errors_never_echo_material() {
        let s = Secret::new("token-material-47").unwrap();
        assert_eq!(format!("{s:?}"), "<secret>");
        assert!(
            !BearerChallenge::parse("Bearer bad\nmaterial")
                .unwrap_err()
                .to_string()
                .contains("material")
        );
    }
    #[test]
    fn ssrf_and_redirect_policy_fail_closed() {
        assert!(SecureUrl::parse("http://169.254.169.254/latest", false).is_err());
        assert!(SecureUrl::parse("https://u:p@example/x", false).is_err());
        let u = SecureUrl::parse("https://client.example/cb", false).unwrap();
        assert!(validate_redirect(&u, "https://evil.example/cb").is_err());
    }
}
