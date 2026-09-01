/// Explicit retrieval limits shared by discovery and JWK refresh.
#[derive(Clone, Debug)]
pub struct RetrievalPolicy {
    /// Maximum metadata document bytes.
    pub max_metadata_bytes: usize,
    /// Maximum JWK-set bytes.
    pub max_jwk_bytes: usize,
    /// Minimum refresh interval.
    pub min_refresh_seconds: u64,
    /// Whether one same-origin redirect is admitted.
    pub same_origin_redirect: bool,
}
impl Default for RetrievalPolicy {
    fn default() -> Self {
        Self {
            max_metadata_bytes: 64 * 1024,
            max_jwk_bytes: 1024 * 1024,
            min_refresh_seconds: 60,
            same_origin_redirect: false,
        }
    }
}
impl RetrievalPolicy {
    /// Derives the exact shared HTTP policy. Cross-origin redirects, proxies, cookies, and ambient credentials stay off.
    pub fn http_policy(&self, cap: usize) -> Policy {
        Policy {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(15),
            total_timeout: Duration::from_secs(30),
            max_request_bytes: 0,
            max_response_bytes: cap,
            max_decompressed_bytes: cap,
            redirects: if self.same_origin_redirect {
                RedirectPolicy::SameOrigin { limit: 1 }
            } else {
                RedirectPolicy::Off
            },
            ..Policy::default()
        }
    }
}

/// Effect injection point; implementations execute GET through `sim-lib-net-http` with the supplied policy.
pub trait DocumentTransport {
    /// Retrieves one exact validated URL without adding credentials.
    fn get(&self, url: &SecureUrl, policy: &Policy) -> Result<Vec<u8>>;
}

/// Retrieval/refresh coordinator with no hidden clock or transport.
pub struct OAuthDocuments<T> {
    transport: T,
    policy: RetrievalPolicy,
    last_jwk_refresh: Option<u64>,
    generation: u64,
}
impl<T: DocumentTransport> OAuthDocuments<T> {
    /// Binds injected transport to explicit policy.
    pub fn new(transport: T, policy: RetrievalPolicy) -> Result<Self> {
        if policy.max_metadata_bytes == 0 || policy.max_jwk_bytes == 0 {
            return Err(OAuthError("OAuth retrieval cap must be non-zero"));
        }
        Ok(Self {
            transport,
            policy,
            last_jwk_refresh: None,
            generation: 0,
        })
    }
    /// Fetches and validates RFC 9728 protected-resource metadata.
    pub fn protected_resource(&self, url: &SecureUrl) -> Result<ProtectedResourceMetadata> {
        let bytes = self.transport.get(
            url,
            &self.policy.http_policy(self.policy.max_metadata_bytes),
        )?;
        validate_document_size(&bytes, self.policy.max_metadata_bytes)?;
        parse_resource(&bytes)
    }
    /// Fetches authorization-server metadata, using OpenID fallback bytes only when OAuth discovery is explicitly absent.
    pub fn authorization_server(
        &self,
        oauth_url: &SecureUrl,
        openid_fallback: Option<(&SecureUrl, &[u8])>,
        expected: &SecureUrl,
    ) -> Result<AuthorizationServerMetadata> {
        let bytes = self.transport.get(
            oauth_url,
            &self.policy.http_policy(self.policy.max_metadata_bytes),
        );
        let bytes = match bytes {
            Ok(v) => v,
            Err(_) => openid_fallback
                .map(|(_, v)| v.to_vec())
                .ok_or(OAuthError("authorization-server discovery failed"))?,
        };
        validate_document_size(&bytes, self.policy.max_metadata_bytes)?;
        let metadata = parse_authorization(&bytes)?;
        metadata.validate(expected, false)?;
        Ok(metadata)
    }
    /// Refreshes a JWK set only when the policy interval has elapsed; callers inject epoch time.
    pub fn jwks(&mut self, url: &SecureUrl, now: u64) -> Result<KeyDocument> {
        if self
            .last_jwk_refresh
            .is_some_and(|last| now < last.saturating_add(self.policy.min_refresh_seconds))
        {
            return Err(OAuthError("JWK refresh attempted before policy interval"));
        }
        let bytes = self
            .transport
            .get(url, &self.policy.http_policy(self.policy.max_jwk_bytes))?;
        validate_document_size(&bytes, self.policy.max_jwk_bytes)?;
        let _: JwkShape =
            serde_json::from_slice(&bytes).map_err(|_| OAuthError("invalid JWK set"))?;
        self.generation += 1;
        self.last_jwk_refresh = Some(now);
        Ok(KeyDocument {
            generation: self.generation,
            json: bytes,
        })
    }
}
#[derive(Deserialize)]
struct RawResource {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct RawAuthorization {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: BTreeSet<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct JwkShape {
    #[serde(rename = "keys")]
    _keys: Vec<serde_json::Value>,
}
fn bounded_extensions(
    values: BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, String>> {
    if values.len() > 32 {
        return Err(OAuthError("too many OAuth metadata extensions"));
    }
    values
        .into_iter()
        .map(|(k, v)| {
            let text = v.to_string();
            if k.len() > 128 || text.len() > 2048 {
                Err(OAuthError("OAuth metadata extension exceeds bound"))
            } else {
                Ok((k, text))
            }
        })
        .collect()
}
fn parse_resource(bytes: &[u8]) -> Result<ProtectedResourceMetadata> {
    let r: RawResource = serde_json::from_slice(bytes)
        .map_err(|_| OAuthError("invalid protected-resource metadata"))?;
    let scopes_supported = ScopeSet::parse(&r.scopes_supported.join(" "))?;
    Ok(ProtectedResourceMetadata {
        resource: SecureUrl::parse(&r.resource, false)?,
        authorization_servers: r
            .authorization_servers
            .iter()
            .map(|v| SecureUrl::parse(v, false))
            .collect::<Result<_>>()?,
        scopes_supported,
        extensions: bounded_extensions(r.extra)?,
    })
}
fn parse_authorization(bytes: &[u8]) -> Result<AuthorizationServerMetadata> {
    let r: RawAuthorization = serde_json::from_slice(bytes)
        .map_err(|_| OAuthError("invalid authorization-server metadata"))?;
    Ok(AuthorizationServerMetadata {
        issuer: SecureUrl::parse(&r.issuer, false)?,
        authorization_endpoint: SecureUrl::parse(&r.authorization_endpoint, false)?,
        token_endpoint: SecureUrl::parse(&r.token_endpoint, false)?,
        registration_endpoint: r
            .registration_endpoint
            .map(|v| SecureUrl::parse(&v, false))
            .transpose()?,
        code_challenge_methods_supported: r.code_challenge_methods_supported,
        extensions: bounded_extensions(r.extra)?,
    })
}
