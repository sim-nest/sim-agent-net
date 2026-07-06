use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use sim_citizen_derive::non_citizen;
use sim_kernel::{
    CapabilityName, CapabilitySet, Cx, DefaultFactory, Error, Expr, GrantSeat, NoopEvalPolicy,
    Object, ObjectCompat, Result, Symbol, Table, Value,
};

use crate::objects::GatewayRequest;

/// Object tag identifying a serialized [`OpenAiGatewayKey`] descriptor.
pub const OPENAI_GATEWAY_KEY_OBJECT: &str = "openai-gateway/key";
const REDACTED_HEADER_VALUE: &str = "[redacted]";

/// An API key registered with the gateway, holding its hashed secret and the
/// capability ceiling it grants.
///
/// The plaintext secret is never stored; the key is identified by the SHA-256
/// hash of the secret and carries the [`CapabilitySet`] that bounds what a
/// request authenticated with it may do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_citizen(
    reason = "gateway key object stores redacted credential policy; serializable projection is openai/GatewayKey descriptor",
    kind = "handle"
)]
pub struct OpenAiGatewayKey {
    id: String,
    key_hash: String,
    capabilities: CapabilitySet,
    default_policy: Expr,
}

impl OpenAiGatewayKey {
    /// Returns a key from a precomputed secret hash and its capability ceiling.
    pub fn new(key_hash: impl Into<String>, capabilities: CapabilitySet) -> Self {
        let key_hash = key_hash.into();
        let id = key_id(&key_hash);
        let default_policy = key_default_policy_expr(&capabilities);
        Self {
            id,
            key_hash,
            capabilities,
            default_policy,
        }
    }

    /// Returns a key built by hashing `secret`, with the given capability ceiling.
    pub fn from_secret(secret: &str, capabilities: CapabilitySet) -> Self {
        Self::new(key_hash(secret), capabilities)
    }

    /// Returns the public key id (a `key_`-prefixed hash fragment).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the hex SHA-256 hash of the key secret.
    pub fn key_hash(&self) -> &str {
        &self.key_hash
    }

    /// Returns the capability ceiling granted by this key.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Returns the default policy expression derived from the capability ceiling.
    pub fn default_policy(&self) -> &Expr {
        &self.default_policy
    }

    /// Returns the key as a serializable map descriptor.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("object", Expr::String(OPENAI_GATEWAY_KEY_OBJECT.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("key-hash", Expr::String(self.key_hash.clone())),
            field("capabilities", capabilities_expr(&self.capabilities)),
            field("default-policy", self.default_policy.clone()),
        ])
    }
}

impl Object for OpenAiGatewayKey {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<openai-gateway-key {}>", self.id))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for OpenAiGatewayKey {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.to_expr())
    }
}

/// Thread-safe table of gateway API keys indexed by secret hash.
///
/// Keys are stored in a SIM table behind a shared mutex so the same table can
/// be cloned across route state; an anonymous capability ceiling applies to
/// requests that present no recognized key.
#[derive(Clone)]
pub struct OpenAiKeyTable {
    inner: Arc<OpenAiKeyTableInner>,
}

struct OpenAiKeyTableInner {
    cx: Mutex<Cx>,
    keys: Value,
    anonymous: CapabilitySet,
}

impl OpenAiKeyTable {
    /// Returns an empty key table with no anonymous capabilities.
    pub fn new() -> Result<Self> {
        Self::with_anonymous(CapabilitySet::new())
    }

    /// Returns an empty key table whose unauthenticated requests get `anonymous`.
    pub fn with_anonymous(anonymous: CapabilitySet) -> Result<Self> {
        let cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
        let keys = cx.factory().table(Vec::new())?;
        Ok(Self {
            inner: Arc::new(OpenAiKeyTableInner {
                cx: Mutex::new(cx),
                keys,
                anonymous,
            }),
        })
    }

    /// Hashes `secret`, registers it with `capabilities`, and returns the new key.
    pub fn add_secret(
        &self,
        secret: &str,
        capabilities: CapabilitySet,
    ) -> Result<OpenAiGatewayKey> {
        let key = OpenAiGatewayKey::from_secret(secret, capabilities);
        self.add_key(key.clone())?;
        Ok(key)
    }

    /// Inserts an already-constructed key, indexing it by its secret hash.
    pub fn add_key(&self, key: OpenAiGatewayKey) -> Result<()> {
        let mut cx = self.cx()?;
        let value = cx.factory().opaque(Arc::new(key.clone()))?;
        table_impl(&self.inner.keys)?.set(&mut cx, Symbol::new(key.key_hash()), value)
    }

    /// Returns the registered key matching `secret`, or `None` if unknown.
    pub fn key_for_secret(&self, secret: &str) -> Result<Option<OpenAiGatewayKey>> {
        self.key_for_hash(&key_hash(secret))
    }

    /// Returns the key authenticating `request`, reading its bearer/api-key header.
    pub fn key_for_request(&self, request: &GatewayRequest) -> Result<Option<OpenAiGatewayKey>> {
        presented_key(request)
            .map(|secret| self.key_for_secret(secret))
            .unwrap_or(Ok(None))
    }

    /// Returns all registered keys.
    pub fn list_keys(&self) -> Result<Vec<OpenAiGatewayKey>> {
        let mut cx = self.cx()?;
        Ok(table_impl(&self.inner.keys)?
            .entries(&mut cx)?
            .into_iter()
            .filter_map(|(_, value)| value.object().downcast_ref::<OpenAiGatewayKey>().cloned())
            .collect())
    }

    /// Returns the capabilities `request` may exercise.
    ///
    /// The result is the key's ceiling (or the anonymous ceiling when no key is
    /// presented), intersected with any capabilities the request body explicitly
    /// requests, so a request can only narrow -- never widen -- its key's grant.
    pub fn effective_capabilities(&self, request: &GatewayRequest) -> Result<CapabilitySet> {
        let ceiling = self
            .key_for_request(request)?
            .map(|key| key.capabilities().clone())
            .unwrap_or_else(|| self.inner.anonymous.clone());
        Ok(requested_capabilities(request)
            .map(|requested| intersect_capabilities(&requested, &ceiling))
            .unwrap_or(ceiling))
    }

    /// Runs `f` with `cx` scoped to the request's effective capabilities.
    pub fn with_effective_capabilities<T>(
        &self,
        cx: &mut Cx,
        request: &GatewayRequest,
        f: impl FnOnce(&mut Cx) -> Result<T>,
    ) -> Result<T> {
        cx.with_capabilities(self.effective_capabilities(request)?, f)
    }

    fn key_for_hash(&self, hash: &str) -> Result<Option<OpenAiGatewayKey>> {
        let mut cx = self.cx()?;
        let value = table_impl(&self.inner.keys)?.get(&mut cx, Symbol::new(hash))?;
        Ok(value.object().downcast_ref::<OpenAiGatewayKey>().cloned())
    }

    fn cx(&self) -> Result<std::sync::MutexGuard<'_, Cx>> {
        self.inner
            .cx
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway key table"))
    }
}

impl Default for OpenAiKeyTable {
    fn default() -> Self {
        Self::new().expect("in-memory SIM key table creation is infallible")
    }
}

/// Returns the process-wide shared gateway key table, initializing it on first use.
pub fn global_openai_key_table() -> &'static OpenAiKeyTable {
    static TABLE: OnceLock<OpenAiKeyTable> = OnceLock::new();
    TABLE.get_or_init(OpenAiKeyTable::default)
}

/// Returns the lowercase hex SHA-256 hash of a key secret.
pub fn key_hash(secret: &str) -> String {
    hex_bytes(&Sha256::digest(secret.as_bytes()))
}

/// Returns a copy of `request` with sensitive auth headers replaced by `[redacted]`.
pub fn redacted_gateway_request(request: &GatewayRequest) -> GatewayRequest {
    GatewayRequest::new(
        request.method().to_owned(),
        request.path().to_owned(),
        redact_headers(request.headers()),
        request.body().to_vec(),
    )
}

/// Grants every capability in `capabilities` to `cx`, through the host-held
/// `seat` minted when `cx` was constructed.
pub fn grant_capability_set(seat: &GrantSeat, cx: &mut Cx, capabilities: &CapabilitySet) {
    capabilities
        .iter()
        .cloned()
        .for_each(|capability| seat.grant(cx, capability));
}

fn key_id(hash: &str) -> String {
    let prefix_len = hash.len().min(12);
    format!("key_{}", &hash[..prefix_len])
}

fn key_default_policy_expr(capabilities: &CapabilitySet) -> Expr {
    Expr::Map(vec![field(
        "capability-ceiling",
        capabilities_expr(capabilities),
    )])
}

fn capabilities_expr(capabilities: &CapabilitySet) -> Expr {
    Expr::Vector(
        capabilities
            .iter()
            .map(|capability| Expr::String(capability.as_str().to_owned()))
            .collect(),
    )
}

fn requested_capabilities(request: &GatewayRequest) -> Option<CapabilitySet> {
    let body = serde_json::from_slice::<JsonValue>(request.body()).ok()?;
    let object = body.as_object()?;
    object
        .get("capabilities")
        .or_else(|| {
            object
                .get("sim")
                .and_then(JsonValue::as_object)
                .and_then(|sim| sim.get("capabilities"))
        })
        .and_then(capability_set_from_json)
}

fn capability_set_from_json(value: &JsonValue) -> Option<CapabilitySet> {
    let mut capabilities = CapabilitySet::new();
    match value {
        JsonValue::String(name) => capabilities.insert(CapabilityName::new(name.clone())),
        JsonValue::Array(items) => {
            for item in items {
                let name = item.as_str()?;
                capabilities.insert(CapabilityName::new(name.to_owned()));
            }
        }
        _ => return None,
    }
    Some(capabilities)
}

fn intersect_capabilities(left: &CapabilitySet, right: &CapabilitySet) -> CapabilitySet {
    let mut capabilities = CapabilitySet::new();
    for capability in left.iter() {
        if right.contains(capability) {
            capabilities.insert(capability.clone());
        }
    }
    capabilities
}

fn presented_key(request: &GatewayRequest) -> Option<&str> {
    for (name, value) in request.headers() {
        if name.eq_ignore_ascii_case("authorization") {
            let value = value.trim();
            if let Some((scheme, token)) = value.split_once(' ')
                && scheme.eq_ignore_ascii_case("bearer")
            {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
        if is_api_key_header(name) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            if is_sensitive_header(name) {
                (name.clone(), REDACTED_HEADER_VALUE.to_owned())
            } else {
                (name.clone(), value.clone())
            }
        })
        .collect()
}

fn is_api_key_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("x-api-key")
        || name.eq_ignore_ascii_case("api-key")
        || name.eq_ignore_ascii_case("openai-api-key")
        || name.eq_ignore_ascii_case("x-openai-api-key")
}

fn is_sensitive_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || is_api_key_header(name)
}

fn table_impl(value: &Value) -> Result<&dyn Table> {
    value.object().as_table_impl().ok_or(Error::TypeMismatch {
        expected: "SIM table",
        found: "non-table",
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

use sim_value::build::entry as field;
