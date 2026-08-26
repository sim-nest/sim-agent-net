/// Algorithms this verifier can explicitly admit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Algorithm {
    /// RSASSA-PKCS1-v1_5 SHA-256.
    Rs256,
    /// ECDSA P-256 SHA-256 fixed-width signature.
    Es256,
    /// Ed25519.
    EdDsa,
}
impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Rs256 => "RS256",
            Self::Es256 => "ES256",
            Self::EdDsa => "EdDSA",
        }
    }
}

/// One immutable JWK-set generation supplied by the HTTP/storage layer.
#[derive(Clone, Debug)]
pub struct KeyDocument {
    /// Monotonic generation chosen by the supplier.
    pub generation: u64,
    /// Serialized bounded JWK set.
    pub json: Vec<u8>,
}

/// Production local verifier policy.
pub struct LocalJwtVerifier {
    keys: KeyDocument,
    algorithms: HashSet<Algorithm>,
    skew: u64,
}
impl LocalJwtVerifier {
    /// Constructs a verifier from injected bytes and a non-empty allowlist.
    pub fn new(
        keys: KeyDocument,
        algorithms: impl IntoIterator<Item = Algorithm>,
        skew_seconds: u64,
    ) -> Result<Self> {
        let algorithms = algorithms.into_iter().collect::<HashSet<_>>();
        if algorithms.is_empty() {
            return Err(OAuthError("JWT algorithm allowlist is empty"));
        }
        if keys.json.len() > 1024 * 1024 {
            return Err(OAuthError("JWK document exceeds policy cap"));
        }
        let _: JwkSet =
            serde_json::from_slice(&keys.json).map_err(|_| OAuthError("invalid JWK set"))?;
        Ok(Self {
            keys,
            algorithms,
            skew: skew_seconds,
        })
    }
    /// Replaces keys only with a newer generation, making rotation explicit.
    pub fn rotate(&mut self, next: KeyDocument) -> Result<()> {
        if next.generation <= self.keys.generation {
            return Err(OAuthError("stale JWK generation"));
        }
        let replacement = Self::new(next, self.algorithms.iter().copied(), self.skew)?;
        self.keys = replacement.keys;
        Ok(())
    }
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
    typ: Option<String>,
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    exp: u64,
    nbf: Option<u64>,
    scope: Option<String>,
    resource: Option<String>,
    jti: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}
impl Audience {
    fn set(self) -> BTreeSet<String> {
        match self {
            Self::One(v) => [v].into(),
            Self::Many(v) => v.into_iter().collect(),
        }
    }
}
#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    n: Option<String>,
    e: Option<String>,
    x: Option<String>,
    y: Option<String>,
    crv: Option<String>,
}

impl AccessTokenVerifier for LocalJwtVerifier {
    fn verify(
        &self,
        token: &Secret,
        expected_issuer: &SecureUrl,
        expected_resource: &SecureUrl,
        required_scopes: &ScopeSet,
        now: u64,
    ) -> Result<VerifiedPrincipal> {
        let parts = token.expose().split('.').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(OAuthError("malformed JWT"));
        }
        let header: Header = decode_json(parts[0])?;
        if header
            .typ
            .as_deref()
            .is_some_and(|v| v != "at+jwt" && v != "JWT")
        {
            return Err(OAuthError("unexpected JWT type"));
        }
        let algorithm = match header.alg.as_str() {
            "RS256" => Algorithm::Rs256,
            "ES256" => Algorithm::Es256,
            "EdDSA" => Algorithm::EdDsa,
            _ => return Err(OAuthError("JWT algorithm is not allowed")),
        };
        if !self.algorithms.contains(&algorithm) {
            return Err(OAuthError("JWT algorithm is not allowed"));
        }
        let set: JwkSet =
            serde_json::from_slice(&self.keys.json).map_err(|_| OAuthError("invalid JWK set"))?;
        let candidates = set
            .keys
            .iter()
            .filter(|k| {
                k.kid == header.kid
                    && k.alg.as_deref().is_none_or(|a| a == algorithm.name())
                    && k.usage.as_deref().is_none_or(|u| u == "sig")
            })
            .collect::<Vec<_>>();
        if header.kid.is_none() || candidates.len() != 1 {
            return Err(OAuthError("JWT key id is missing or ambiguous"));
        }
        let message = format!("{}.{}", parts[0], parts[1]);
        let sig = decode(parts[2])?;
        verify_key(candidates[0], algorithm, message.as_bytes(), &sig)?;
        let claims: Claims = decode_json(parts[1])?;
        if claims.iss != expected_issuer.as_str() {
            return Err(OAuthError("token issuer mismatch"));
        }
        if claims.exp.saturating_add(self.skew) < now {
            return Err(OAuthError("token expired"));
        }
        if claims
            .nbf
            .is_some_and(|v| v > now.saturating_add(self.skew))
        {
            return Err(OAuthError("token not yet valid"));
        }
        let audience = claims.aud.set();
        if !audience.contains(expected_resource.as_str()) {
            return Err(OAuthError("token audience mismatch"));
        }
        if claims.resource.as_deref() != Some(expected_resource.as_str()) {
            return Err(OAuthError("token resource mismatch"));
        }
        let scopes = ScopeSet::parse(claims.scope.as_deref().unwrap_or(""))?;
        if !scopes.contains_all(required_scopes) {
            return Err(OAuthError("insufficient token scope"));
        }
        VerifiedPrincipal::new(
            expected_issuer.clone(),
            claims.sub,
            audience,
            expected_resource.clone(),
            scopes,
            claims.exp,
            claims.jti,
        )
    }
}
fn decode(v: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(v)
        .map_err(|_| OAuthError("invalid JWT encoding"))
}
fn decode_json<T: for<'a> Deserialize<'a>>(v: &str) -> Result<T> {
    let b = decode(v)?;
    serde_json::from_slice(&b).map_err(|_| OAuthError("invalid JWT JSON"))
}
fn verify_key(key: &Jwk, alg: Algorithm, message: &[u8], sig: &[u8]) -> Result<()> {
    let result = match alg {
        Algorithm::Rs256 if key.kty == "RSA" => {
            let n = decode(
                key.n
                    .as_deref()
                    .ok_or(OAuthError("RSA key is incomplete"))?,
            )?;
            let e = decode(
                key.e
                    .as_deref()
                    .ok_or(OAuthError("RSA key is incomplete"))?,
            )?;
            signature::RsaPublicKeyComponents { n: &n, e: &e }.verify(
                &signature::RSA_PKCS1_2048_8192_SHA256,
                message,
                sig,
            )
        }
        Algorithm::Es256 if key.kty == "EC" && key.crv.as_deref() == Some("P-256") => {
            let mut p = vec![4];
            p.extend(decode(
                key.x.as_deref().ok_or(OAuthError("EC key is incomplete"))?,
            )?);
            p.extend(decode(
                key.y.as_deref().ok_or(OAuthError("EC key is incomplete"))?,
            )?);
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, p)
                .verify(message, sig)
        }
        Algorithm::EdDsa if key.kty == "OKP" && key.crv.as_deref() == Some("Ed25519") => {
            let x = decode(
                key.x
                    .as_deref()
                    .ok_or(OAuthError("Ed25519 key is incomplete"))?,
            )?;
            signature::UnparsedPublicKey::new(&signature::ED25519, x).verify(message, sig)
        }
        _ => return Err(OAuthError("JWK type does not match JWT algorithm")),
    };
    result.map_err(|_| OAuthError("JWT signature verification failed"))
}
