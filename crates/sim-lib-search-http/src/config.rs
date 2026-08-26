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
