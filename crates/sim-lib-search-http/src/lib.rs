//! Provider-neutral HTTP execution for [`SearchWireCodec`] implementations.
//!
//! This crate owns policy and effects, never provider syntax. Configuration is
//! supplied as a checked [`ConfigTable`], credentials remain opaque references,
//! and live bytes cross only the secret resolver-to-request boundary.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sim_config::{ConfigTable, ConfigView};
use sim_kernel::{
    CapabilityName, ContentId, Cx, Datum, Error, Expr, Result as SimResult, Symbol, Value,
};
use sim_lib_net_http::{
    Client, Connector, Header, Method, Policy as HttpPolicy, Request, RequestBody, Url,
};
use sim_lib_search_core::{
    ProviderClaim, SearchError, SearchObservation, SearchPage, SearchQuery, SearchSite,
    SearchWireCodec,
};
use sim_lib_skill::{
    SkillCacheMode, SkillCard, SkillCassetteMode, SkillEventSink, SkillPolicy, SkillRole,
    SkillTransport, skill_specific_call_capability,
};
use sim_lib_web_core::DecodeLimits;
use sim_shape::{AnyShape, ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape, shape_value};

/// Network-free cookbook descriptors embedded for discovery.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

/// Secret reference understood only by the injected resolver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrincipalRef(pub String);

/// S1 boundary: implementations return header bytes without making them durable.
pub trait SecretResolver: Send + Sync {
    fn principal_headers(
        &self,
        principal: &PrincipalRef,
    ) -> Result<Vec<(String, String)>, SearchHttpError>;
}

/// Shared HTTP membrane seam; tests and host capsules inject an implementation.
pub trait SearchHttpClient: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, SearchHttpError>;
}

/// Request admitted by the transport policy.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub response_limit: usize,
}
/// Raw response captured before provider decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Production adapter over the shared `sim-lib-net-http` membrane.
pub struct MembraneHttpClient<C> {
    connector: C,
    policy: HttpPolicy,
}
impl<C> MembraneHttpClient<C> {
    pub fn new(connector: C, policy: HttpPolicy) -> Self {
        Self { connector, policy }
    }
}
impl<C: Connector + Clone> SearchHttpClient for MembraneHttpClient<C> {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, SearchHttpError> {
        let mut policy = self.policy.clone();
        policy.max_response_bytes = policy.max_response_bytes.min(request.response_limit);
        policy.connect_timeout = policy.connect_timeout.min(request.timeout);
        policy.read_timeout = policy.read_timeout.min(request.timeout);
        let client = Client::new(self.connector.clone(), policy);
        let headers = request
            .headers
            .iter()
            .map(|(n, v)| Header::new(n, v).map_err(wire))
            .collect::<Result<Vec<_>, _>>()?;
        let response = client
            .execute(Request {
                method: Method::new("POST").map_err(wire)?,
                url: Url::parse(&request.endpoint).map_err(wire)?,
                headers,
                body: RequestBody::Bytes(&request.body),
                deadline: None,
                cancellation: Default::default(),
            })
            .map_err(wire)?;
        Ok(HttpResponse {
            status: response.status,
            headers: response
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name().into(),
                        if h.is_sensitive() {
                            "[REDACTED]".into()
                        } else {
                            h.value().into()
                        },
                    )
                })
                .collect(),
            body: response.into_body(),
        })
    }
}
fn wire(error: sim_lib_net_http::Error) -> SearchHttpError {
    SearchHttpError::Transport(error.to_string())
}

/// Explicit per-site limits. Zero and unbounded values are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteLimits {
    pub concurrent_requests: usize,
    pub minimum_interval: Duration,
    pub timeout: Duration,
    pub response_bytes: usize,
    pub pages: usize,
    pub egress_bytes: usize,
    pub query_timeout: Duration,
}
impl SiteLimits {
    fn validate(&self) -> Result<(), SearchHttpError> {
        if self.concurrent_requests == 0
            || self.timeout.is_zero()
            || self.response_bytes == 0
            || self.pages == 0
            || self.egress_bytes == 0
            || self.query_timeout.is_zero()
        {
            return Err(SearchHttpError::Policy(
                "every site limit must be explicit and non-zero".into(),
            ));
        }
        Ok(())
    }
}

/// Public, redacted site configuration resolved from `sim-config`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSiteConfig {
    pub site_id: String,
    pub endpoint: String,
    pub codec_id: String,
    pub config_revision: String,
    pub principal: PrincipalRef,
    pub limits: SiteLimits,
    pub observation_ttl: Duration,
}
impl SearchSiteConfig {
    /// Reads a site from a `sim-config` table. Secret material is not accepted.
    pub fn from_config(table: &ConfigTable) -> Result<Self, SearchHttpError> {
        let v = ConfigView::new(table);
        let positive = |key| -> Result<usize, SearchHttpError> {
            let n = v.required_i64(key).map_err(config)?;
            usize::try_from(n)
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| SearchHttpError::Config(format!("{key} must be positive")))
        };
        let millis = |key| positive(key).map(|n| Duration::from_millis(n as u64));
        let config = Self {
            site_id: v.required_string("site-id").map_err(config)?.into(),
            endpoint: v.required_string("endpoint").map_err(config)?.into(),
            codec_id: v.required_string("codec-id").map_err(config)?.into(),
            config_revision: v.required_string("config-revision").map_err(config)?.into(),
            principal: PrincipalRef(v.required_string("principal-ref").map_err(config)?.into()),
            limits: SiteLimits {
                concurrent_requests: positive("concurrent-requests")?,
                minimum_interval: millis("minimum-interval-ms")?,
                timeout: millis("timeout-ms")?,
                response_bytes: positive("response-bytes")?,
                pages: positive("page-count")?,
                egress_bytes: positive("egress-bytes")?,
                query_timeout: millis("query-timeout-ms")?,
            },
            observation_ttl: millis("observation-ttl-ms")?,
        };
        config.limits.validate()?;
        Ok(config)
    }
}
fn config(error: sim_config::ConfigError) -> SearchHttpError {
    SearchHttpError::Config(error.to_string())
}

/// Typed transport/provider observations. They are safe to persist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchHttpNotice {
    HttpStatus(u16),
    Provider(String),
    PartialPage { surviving_rows: usize },
    Throttled,
    CacheHit,
    CassetteReplay,
}
/// Raw response identity and bounded bytes retained before decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawResponseCapture {
    pub id: ContentId,
    pub status: u16,
    pub body: Vec<u8>,
}
/// Complete effect receipt; never contains headers or credential bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHttpReceipt {
    pub site_id: String,
    pub config_revision: String,
    pub codec_id: String,
    pub captures: Vec<RawResponseCapture>,
    pub notices: Vec<SearchHttpNotice>,
    pub pages: Vec<SearchPage>,
}

/// Live/record/replay/offline selection for direct transport calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallMode {
    Live,
    Record,
    Replay,
    Offline,
}

/// Transport failure without secret-bearing request material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchHttpError {
    Config(String),
    Policy(String),
    Capability(String),
    Transport(String),
    Provider(u16),
    Decode(String),
    CassetteMiss,
    Poisoned,
}
impl fmt::Display for SearchHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SearchHttpError {}

#[derive(Clone)]
struct CapabilityObservation {
    value: Datum,
    expires: Instant,
}
#[derive(Default)]
struct State {
    active: usize,
    last_request: Option<Instant>,
    capabilities: BTreeMap<String, CapabilityObservation>,
    cassettes: BTreeMap<String, SearchHttpReceipt>,
    cache: BTreeMap<String, SearchHttpReceipt>,
}

/// Provider-neutral HTTP realization of one search wire codec.
pub struct HttpSearchTransport<C: SearchWireCodec> {
    id: String,
    codec: C,
    config: SearchSiteConfig,
    http: Arc<dyn SearchHttpClient>,
    secrets: Arc<dyn SecretResolver>,
    state: Mutex<State>,
    decode_limits: DecodeLimits,
}
impl<C: SearchWireCodec> HttpSearchTransport<C> {
    pub fn new(
        id: impl Into<String>,
        codec: C,
        config: SearchSiteConfig,
        http: Arc<dyn SearchHttpClient>,
        secrets: Arc<dyn SecretResolver>,
        decode_limits: DecodeLimits,
    ) -> Result<Self, SearchHttpError> {
        config.limits.validate()?;
        if config.codec_id != codec.codec_id() {
            return Err(SearchHttpError::Config(
                "configured codec id does not match codec".into(),
            ));
        }
        Ok(Self {
            id: id.into(),
            codec,
            config,
            http,
            secrets,
            state: Mutex::new(State::default()),
            decode_limits,
        })
    }
    /// Explicit config discovery; never encodes or submits an empty search.
    pub fn discover_config(&self, now: Instant) -> Result<Datum, SearchHttpError> {
        let key = format!("{}:{}", self.config.site_id, self.config.config_revision);
        if let Some(hit) = self
            .state
            .lock()
            .map_err(|_| SearchHttpError::Poisoned)?
            .capabilities
            .get(&key)
            .filter(|v| v.expires > now)
            .cloned()
        {
            return Ok(hit.value);
        }
        let value = Datum::Node {
            tag: Symbol::qualified("search-http", "site"),
            fields: vec![
                (
                    Symbol::new("site-id"),
                    Datum::String(self.config.site_id.clone()),
                ),
                (
                    Symbol::new("codec-id"),
                    Datum::String(self.config.codec_id.clone()),
                ),
                (
                    Symbol::new("config-revision"),
                    Datum::String(self.config.config_revision.clone()),
                ),
            ],
        };
        self.state
            .lock()
            .map_err(|_| SearchHttpError::Poisoned)?
            .capabilities
            .insert(
                key,
                CapabilityObservation {
                    value: value.clone(),
                    expires: now + self.config.observation_ttl,
                },
            );
        Ok(value)
    }
    /// Explicit health operation; it inspects policy/config state without HTTP.
    pub fn health_observation(&self) -> Datum {
        Datum::Node {
            tag: Symbol::qualified("search-http", "health"),
            fields: vec![
                (
                    Symbol::new("site-id"),
                    Datum::String(self.config.site_id.clone()),
                ),
                (
                    Symbol::new("status"),
                    Datum::Symbol(Symbol::new("configured")),
                ),
            ],
        }
    }
    /// Executes bounded pages or replays the stable receipt.
    pub fn search(
        &self,
        query: SearchQuery,
        mode: CallMode,
    ) -> Result<SearchHttpReceipt, SearchHttpError> {
        let key = self.call_key(&query);
        {
            let state = self.state.lock().map_err(|_| SearchHttpError::Poisoned)?;
            if let Some(hit) = state.cache.get(&key) {
                return Ok(hit.clone());
            }
            if matches!(mode, CallMode::Replay | CallMode::Offline) {
                return state
                    .cassettes
                    .get(&key)
                    .cloned()
                    .ok_or(SearchHttpError::CassetteMiss);
            }
        }
        let started = Instant::now();
        let mut continuation_query = query.clone();
        let mut receipt = SearchHttpReceipt {
            site_id: self.config.site_id.clone(),
            config_revision: self.config.config_revision.clone(),
            codec_id: self.codec.codec_id().into(),
            captures: vec![],
            notices: vec![],
            pages: vec![],
        };
        let mut egress = 0usize;
        for _ in 0..self.config.limits.pages {
            if started.elapsed() >= self.config.limits.query_timeout {
                return Err(SearchHttpError::Policy(
                    "overall query budget exhausted".into(),
                ));
            }
            let body = self
                .codec
                .encode_request(&continuation_query, self.decode_limits)
                .map_err(decode)?;
            egress = egress
                .checked_add(body.len())
                .ok_or_else(|| SearchHttpError::Policy("egress overflow".into()))?;
            if egress > self.config.limits.egress_bytes {
                return Err(SearchHttpError::Policy(
                    "query egress limit exceeded".into(),
                ));
            }
            self.admit_request()?;
            let headers = self.secrets.principal_headers(&self.config.principal)?;
            let response = self.http.execute(HttpRequest {
                endpoint: self.config.endpoint.clone(),
                headers,
                body,
                timeout: self.config.limits.timeout.min(
                    self.config
                        .limits
                        .query_timeout
                        .saturating_sub(started.elapsed()),
                ),
                response_limit: self.config.limits.response_bytes,
            });
            self.release_request();
            let response = response?;
            if response.body.len() > self.config.limits.response_bytes {
                return Err(SearchHttpError::Policy(
                    "response byte limit exceeded".into(),
                ));
            }
            let capture = RawResponseCapture {
                id: Datum::Bytes(response.body.clone())
                    .content_id()
                    .map_err(|e| SearchHttpError::Decode(e.to_string()))?,
                status: response.status,
                body: response.body.clone(),
            };
            receipt.captures.push(capture.clone());
            if !(200..300).contains(&response.status) {
                receipt
                    .notices
                    .push(SearchHttpNotice::HttpStatus(response.status));
                return Err(SearchHttpError::Provider(response.status));
            }
            let page = self
                .codec
                .decode_response(&capture.body, &continuation_query, self.decode_limits)
                .map_err(decode)?;
            let continuation = page.continuation.clone();
            receipt.pages.push(page);
            let Some(token) = continuation else { break };
            continuation_query.text = format!("{}\ncontinuation:{token}", query.text);
        }
        let mut state = self.state.lock().map_err(|_| SearchHttpError::Poisoned)?;
        state.cache.insert(key.clone(), receipt.clone());
        if mode == CallMode::Record {
            state.cassettes.insert(key, receipt.clone());
        }
        Ok(receipt)
    }
    fn admit_request(&self) -> Result<(), SearchHttpError> {
        let mut s = self.state.lock().map_err(|_| SearchHttpError::Poisoned)?;
        if s.active >= self.config.limits.concurrent_requests {
            return Err(SearchHttpError::Policy(
                "concurrent request limit exceeded".into(),
            ));
        }
        if s.last_request
            .is_some_and(|last| last.elapsed() < self.config.limits.minimum_interval)
        {
            return Err(SearchHttpError::Policy(
                "minimum request interval not elapsed".into(),
            ));
        }
        s.active += 1;
        s.last_request = Some(Instant::now());
        Ok(())
    }
    fn release_request(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.active = s.active.saturating_sub(1);
        }
    }
    fn call_key(&self, query: &SearchQuery) -> String {
        format!(
            "search-http-v1;site={};revision={};codec={}:{};query={:?}",
            self.config.site_id,
            self.config.config_revision,
            self.codec.codec_id(),
            self.codec.codec_version(),
            query.to_datum()
        )
    }
    fn card(&self) -> SkillCard {
        let id = format!("search.{}", self.config.site_id);
        SkillCard {
            id: id.clone(),
            symbol: Symbol::qualified("search", self.config.site_id.clone()),
            aliases: vec![],
            origin: Symbol::qualified("search-http", "site"),
            title: format!("Search {}", self.config.site_id),
            description: format!(
                "Provider-neutral search through codec {} at configured site {} revision {}",
                self.codec.codec_id(),
                self.config.endpoint,
                self.config.config_revision
            ),
            input_shape: query_shape(&id),
            output_shape: page_shape(&id),
            roles: vec![SkillRole::Retriever, SkillRole::Tool],
            capabilities: vec![
                CapabilityName::new("net/http"),
                skill_specific_call_capability(&id),
            ],
            policy: SkillPolicy::default().with_search_defaults(),
            transport_id: self.id.clone(),
            transport_kind: "search-http".into(),
            operation: "search".into(),
        }
    }
}
fn decode(error: SearchError) -> SearchHttpError {
    SearchHttpError::Decode(error.to_string())
}

trait SearchPolicyExt {
    fn with_search_defaults(self) -> Self;
}
impl SearchPolicyExt for SkillPolicy {
    fn with_search_defaults(mut self) -> Self {
        self.idempotent = true;
        self.cache = SkillCacheMode::ReadThrough;
        self.cassette = SkillCassetteMode::RecordReplay;
        self
    }
}

impl<C: SearchWireCodec + Send + Sync> SkillTransport for HttpSearchTransport<C> {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> &str {
        "search-http"
    }
    fn discover(&self, _cx: &mut Cx) -> SimResult<Vec<SkillCard>> {
        Ok(vec![self.card()])
    }
    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> SimResult<Value> {
        if card.operation != "search" {
            return Err(Error::Eval("unsupported search site operation".into()));
        }
        let query = query_from_expr(args.object().as_expr(cx)?)?;
        let receipt = self
            .search(query, CallMode::Live)
            .map_err(|e| Error::Eval(e.to_string()))?;
        page_value(
            cx,
            receipt
                .pages
                .last()
                .ok_or_else(|| Error::Eval("search returned no page".into()))?,
        )
    }
    fn health(&self, cx: &mut Cx) -> SimResult<Value> {
        cx.factory().expr(datum_expr(&self.health_observation()))
    }
}

fn query_shape(id: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("search-http", format!("{id}-query")),
        Arc::new(ListShape::new(vec![Arc::new(FieldShape::anonymous(vec![
            FieldSpec::required(
                Symbol::new("text"),
                Arc::new(ExprKindShape::new(ExprKind::String)),
            ),
            FieldSpec::required(
                Symbol::new("limit"),
                Arc::new(ExprKindShape::new(ExprKind::Number)),
            ),
        ]))])),
    )
}
fn page_shape(id: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("search-http", format!("{id}-page")),
        Arc::new(FieldShape::anonymous(vec![FieldSpec::required(
            Symbol::new("observations"),
            Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
        )])),
    )
}
fn query_from_expr(expr: Expr) -> SimResult<SearchQuery> {
    let Expr::List(mut args) = expr else {
        return Err(Error::Eval("search expects one query table".into()));
    };
    if args.len() != 1 {
        return Err(Error::Eval("search expects one query table".into()));
    }
    let Expr::Map(fields) = args.remove(0) else {
        return Err(Error::Eval("search query must be a table".into()));
    };
    let get = |name: &str| {
        fields
            .iter()
            .find_map(|(k, v)| matches!(k, Expr::Symbol(s) if s.name.as_ref() == name).then_some(v))
    };
    let text = match get("text") {
        Some(Expr::String(s)) => s.clone(),
        _ => return Err(Error::Eval("search query text must be a string".into())),
    };
    let limit = match get("limit") {
        Some(Expr::Number(n)) => n
            .canonical
            .parse()
            .map_err(|_| Error::Eval("search limit must be u32".into()))?,
        _ => return Err(Error::Eval("search query limit must be a number".into())),
    };
    SearchQuery::checked(text, Vec::<SearchSite>::new(), None, limit)
        .map_err(|e| Error::Eval(e.to_string()))
}
fn page_value(cx: &mut Cx, page: &SearchPage) -> SimResult<Value> {
    let observations = cx.factory().list(
        page.observations
            .iter()
            .map(|o| cx.factory().expr(observation_expr(o)))
            .collect::<SimResult<Vec<_>>>()?,
    )?;
    cx.factory().table(vec![
        (Symbol::new("observations"), observations),
        (
            Symbol::new("continuation"),
            match &page.continuation {
                Some(v) => cx.factory().string(v.clone())?,
                None => cx.factory().nil()?,
            },
        ),
    ])
}
fn observation_expr(o: &SearchObservation) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("retrieval-uri")),
            Expr::String(o.retrieval_uri.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("claim")),
            o.claim.as_ref().map(claim_expr).unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("capture-id")),
            o.capture_id
                .as_ref()
                .map(|id| Expr::String(id.bytes.iter().map(|byte| format!("{byte:02x}")).collect()))
                .unwrap_or(Expr::Nil),
        ),
    ])
}
fn claim_expr(c: &ProviderClaim) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("provider")),
            Expr::String(c.provider.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("uri")),
            Expr::String(c.uri.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("title")),
            c.title.clone().map(Expr::String).unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("snippet")),
            c.snippet.clone().map(Expr::String).unwrap_or(Expr::Nil),
        ),
    ])
}
fn datum_expr(d: &Datum) -> Expr {
    match d {
        Datum::Nil => Expr::Nil,
        Datum::Bool(v) => Expr::Bool(*v),
        Datum::String(v) => Expr::String(v.clone()),
        Datum::Bytes(v) => Expr::Bytes(v.clone()),
        Datum::Symbol(v) => Expr::Symbol(v.clone()),
        Datum::Number(v) => Expr::Number(v.clone()),
        Datum::List(v) => Expr::List(v.iter().map(datum_expr).collect()),
        Datum::Vector(v) => Expr::Vector(v.iter().map(datum_expr).collect()),
        Datum::Set(v) => Expr::Set(v.iter().map(datum_expr).collect()),
        Datum::Map(v) => Expr::Map(
            v.iter()
                .map(|(k, v)| (datum_expr(k), datum_expr(v)))
                .collect(),
        ),
        Datum::Node { tag, fields } => Expr::Map(
            std::iter::once((Expr::Symbol(Symbol::new("kind")), Expr::Symbol(tag.clone())))
                .chain(
                    fields
                        .iter()
                        .map(|(k, v)| (Expr::Symbol(k.clone()), datum_expr(v))),
                )
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests;
