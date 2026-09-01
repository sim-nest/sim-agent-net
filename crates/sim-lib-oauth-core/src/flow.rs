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
