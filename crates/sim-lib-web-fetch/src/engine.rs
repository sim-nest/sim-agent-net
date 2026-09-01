//! Policy-gated, content-addressed web capture.
#![forbid(unsafe_code)]

use std::{collections::BTreeMap, fmt, sync::Arc};

use sim_kernel::{CapabilityName, ContentId, Cx, Datum};
use sim_lib_agent_runner_core::fenced_data_text_for_id;
use sim_lib_net_http::{
    Client, Connector, Header, Method, Policy as HttpPolicy, Request, RequestBody, Url,
};
use sim_lib_web_core::{DecodeLimits, WebCapture, WebExchange, WebRepresentation};

use crate::projection::project;
pub use crate::store::MemoryCaptureDir;

const ROBOTS_TTL_SECS: u64 = 24 * 60 * 60;

/// Cache/network selection is explicit at every call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchMode {
    Live,
    PreferCache,
    Offline { capture: ContentId },
    Revalidate,
}

/// Bounded, fully authorized capture request.
#[derive(Clone, Debug)]
pub struct FetchPlan {
    pub url: String,
    pub method: String,
    pub mode: FetchMode,
    pub user_agent: String,
    pub now_secs: u64,
    pub max_redirects: usize,
    pub max_body_bytes: usize,
    pub max_origins: usize,
    pub max_requests: usize,
    pub max_requests_per_origin: usize,
    pub extractor_version: String,
}
impl FetchPlan {
    pub fn get(url: impl Into<String>, mode: FetchMode) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            mode,
            user_agent: "sim-web-fetch".into(),
            now_secs: 0,
            max_redirects: 5,
            max_body_bytes: 2 * 1024 * 1024,
            max_origins: 8,
            max_requests: 16,
            max_requests_per_origin: 4,
            extractor_version: "1".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepresentationOutcome {
    Decoded,
    UnsupportedRepresentation { media_type: Option<String> },
}

#[derive(Clone, Debug)]
pub struct FetchReceipt {
    pub capture: WebCapture,
    pub representation: Option<WebRepresentation>,
    pub outcome: RepresentationOutcome,
    pub policy: PolicyReceipt,
    pub robots: Vec<RobotsReceipt>,
    pub exchange_chain: Vec<ExchangeReceipt>,
    pub from_cache: bool,
}
impl FetchReceipt {
    pub fn fenced_text(&self) -> Result<Option<String>, FetchError> {
        let Some(rep) = &self.representation else {
            return Ok(None);
        };
        Ok(Some(fenced_data_text_for_id(
            "web-capture",
            &rep.text,
            &rep.content_id,
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyReceipt {
    pub method: String,
    pub origins: Vec<String>,
    pub requests: usize,
    pub decision: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotsReceipt {
    pub origin: String,
    pub status: Option<u16>,
    pub decision: String,
    pub expires_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeReceipt {
    pub url: String,
    pub status: u16,
    pub location: Option<String>,
    pub sensitive_headers_redacted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Capability(String),
    InvalidPlan(String),
    PolicyDenied(String),
    RobotsDenied(String),
    RateLimited(String),
    OfflineMiss(ContentId),
    Transport(String),
    Storage(String),
    Decode(String),
    Fence(String),
}
impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FetchError {}

/// Caller-supplied `Dir`-shaped immutable evidence store. Implementations may
/// adapt a kernel `Dir`; the capture owner never opens ambient storage.
pub trait CaptureDir: Send + Sync {
    fn capture(&self, id: &ContentId) -> Result<Option<StoredCapture>, FetchError>;
    fn put_capture(&self, capture: StoredCapture) -> Result<(), FetchError>;
    fn url_capture(&self, url: &str) -> Result<Option<ContentId>, FetchError>;
    fn point_url(&self, url: &str, id: &ContentId) -> Result<(), FetchError>;
    fn robots(&self, origin: &str) -> Result<Option<StoredRobots>, FetchError>;
    fn put_robots(&self, origin: &str, robots: StoredRobots) -> Result<(), FetchError>;
}

#[derive(Clone, Debug)]
pub struct StoredCapture {
    pub receipt: FetchReceipt,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}
#[derive(Clone, Debug)]
pub struct StoredRobots {
    pub body: Vec<u8>,
    pub status: Option<u16>,
    pub decision: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub trait HttpExecutor: Send + Sync {
    fn execute(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body_cap: usize,
    ) -> Result<HttpResponse, FetchError>;
}

/// Production adapter over the delivered `sim-lib-net-http` membrane.
pub struct NetHttpExecutor<C> {
    connector: C,
    policy: HttpPolicy,
}
impl<C> NetHttpExecutor<C> {
    pub fn new(connector: C, policy: HttpPolicy) -> Self {
        Self { connector, policy }
    }
}
impl<C: Connector + Clone> HttpExecutor for NetHttpExecutor<C> {
    fn execute(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body_cap: usize,
    ) -> Result<HttpResponse, FetchError> {
        let mut policy = self.policy.clone();
        policy.max_response_bytes = policy.max_response_bytes.min(body_cap);
        policy.redirects = sim_lib_net_http::RedirectPolicy::Off;
        let client = Client::new(self.connector.clone(), policy);
        let request = Request {
            method: Method::new(method).map_err(net)?,
            url: Url::parse(url).map_err(net)?,
            headers: headers
                .iter()
                .map(|(n, v)| Header::new(n, v).map_err(net))
                .collect::<Result<_, _>>()?,
            body: RequestBody::Empty,
            deadline: None,
            cancellation: Default::default(),
        };
        let response = client.execute(request).map_err(net)?;
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
fn net(e: sim_lib_net_http::Error) -> FetchError {
    FetchError::Transport(e.to_string())
}

/// Independent landing-page egress decision. Search authorization is never
/// accepted as capture authorization.
pub trait EgressPolicy: Send + Sync {
    fn authorize(&self, method: &str, url: &Url) -> Result<(), FetchError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PublicWebEgress;
impl EgressPolicy for PublicWebEgress {
    fn authorize(&self, method: &str, url: &Url) -> Result<(), FetchError> {
        if !matches!(method, "GET" | "HEAD") {
            return Err(FetchError::PolicyDenied("unsafe method".into()));
        }
        if !matches!(url.port(), 80 | 443) {
            return Err(FetchError::PolicyDenied("port denied".into()));
        }
        let host = url.host().trim_matches(['[', ']']).to_ascii_lowercase();
        if host == "localhost" || host.ends_with(".localhost") {
            return Err(FetchError::PolicyDenied("local host denied".into()));
        }
        if let Ok(ip) = host.parse::<std::net::IpAddr>()
            && (ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || match ip {
                    std::net::IpAddr::V4(v) => v.is_private() || v.is_link_local(),
                    std::net::IpAddr::V6(v) => v.is_unique_local() || v.is_unicast_link_local(),
                })
        {
            return Err(FetchError::PolicyDenied("non-public address denied".into()));
        }
        Ok(())
    }
}

pub struct WebFetcher {
    transport: Arc<dyn HttpExecutor>,
    dir: Arc<dyn CaptureDir>,
    egress: Arc<dyn EgressPolicy>,
}
impl WebFetcher {
    pub fn new(
        transport: Arc<dyn HttpExecutor>,
        dir: Arc<dyn CaptureDir>,
        egress: Arc<dyn EgressPolicy>,
    ) -> Self {
        Self {
            transport,
            dir,
            egress,
        }
    }

    pub fn capture(&self, cx: &mut Cx, plan: FetchPlan) -> Result<FetchReceipt, FetchError> {
        validate_plan(&plan)?;
        if let FetchMode::Offline { capture } = &plan.mode {
            return self
                .dir
                .capture(capture)?
                .map(|v| {
                    let mut r = v.receipt;
                    r.from_cache = true;
                    r
                })
                .ok_or_else(|| FetchError::OfflineMiss(capture.clone()));
        }
        require_net_http(cx)?;
        if matches!(plan.mode, FetchMode::PreferCache)
            && let Some(id) = self.dir.url_capture(&plan.url)?
            && let Some(stored) = self.dir.capture(&id)?
        {
            let mut r = stored.receipt;
            r.from_cache = true;
            return Ok(r);
        }

        let mut current = plan.url.clone();
        let mut chain = Vec::new();
        let mut robots_receipts = Vec::new();
        let mut origins = Vec::new();
        let mut requests = 0usize;
        let mut origin_requests = BTreeMap::<String, usize>::new();
        let mut validators = Vec::new();
        if matches!(plan.mode, FetchMode::Revalidate)
            && let Some(id) = self.dir.url_capture(&plan.url)?
            && let Some(old) = self.dir.capture(&id)?
        {
            if let Some(v) = old.etag {
                validators.push(("If-None-Match".into(), v));
            }
            if let Some(v) = old.last_modified {
                validators.push(("If-Modified-Since".into(), v));
            }
        }
        let response = loop {
            let url = Url::parse(&current).map_err(net)?;
            self.egress.authorize(&plan.method, &url)?;
            let origin = origin(&url);
            if !origins.contains(&origin) {
                if origins.len() >= plan.max_origins {
                    return Err(FetchError::InvalidPlan("origin budget exceeded".into()));
                }
                origins.push(origin.clone());
                self.check_robots(&plan, &url, &mut requests, &mut robots_receipts)?;
            }
            let bucket = origin_requests.entry(origin.clone()).or_default();
            *bucket += 1;
            if *bucket > plan.max_requests_per_origin {
                return Err(FetchError::RateLimited(origin));
            }
            requests += 1;
            if requests > plan.max_requests {
                return Err(FetchError::InvalidPlan("request budget exceeded".into()));
            }
            let response =
                self.transport
                    .execute(&plan.method, &current, &validators, plan.max_body_bytes)?;
            let location = response.header("location").map(str::to_owned);
            chain.push(ExchangeReceipt {
                url: current.clone(),
                status: response.status,
                location: location.clone(),
                sensitive_headers_redacted: true,
            });
            if response.status == 304 {
                let id = self
                    .dir
                    .url_capture(&plan.url)?
                    .ok_or_else(|| FetchError::Storage("304 without cached capture".into()))?;
                let mut old = self
                    .dir
                    .capture(&id)?
                    .ok_or_else(|| FetchError::Storage("validator index is dangling".into()))?
                    .receipt;
                old.from_cache = true;
                old.exchange_chain.extend(chain);
                return Ok(old);
            }
            if (300..400).contains(&response.status) {
                let next = location
                    .ok_or_else(|| FetchError::Transport("redirect without location".into()))?;
                if chain.len() > plan.max_redirects {
                    return Err(FetchError::PolicyDenied("redirect budget exceeded".into()));
                }
                current = resolve_location(&url, &next)?;
                validators.clear();
                continue;
            }
            break response;
        };
        let media = media_type(response.header("content-type"));
        let raw_id = Datum::Bytes(response.body.clone())
            .content_id()
            .map_err(|e| FetchError::Storage(e.to_string()))?;
        let retrieval_uri = sim_lib_net_core::normalize_retrieval_uri(&current)
            .map_err(|e| FetchError::InvalidPlan(e.to_string()))?;
        let capture = WebCapture::checked(
            retrieval_uri,
            raw_id.clone(),
            response.body.clone(),
            WebExchange {
                method: plan.method.clone(),
                status: response.status,
                final_uri: current.clone(),
                media_type: media.clone(),
                received_bytes: response.body.len() as u64,
            },
            DecodeLimits {
                max_body_bytes: plan.max_body_bytes,
                ..Default::default()
            },
        )
        .map_err(|e| FetchError::Decode(e.to_string()))?;
        let (representation, outcome) =
            project(&capture, &plan.extractor_version, media.as_deref())?;
        let receipt = FetchReceipt {
            capture,
            representation,
            outcome,
            policy: PolicyReceipt {
                method: plan.method,
                origins,
                requests,
                decision: "allowed".into(),
            },
            robots: robots_receipts,
            exchange_chain: chain,
            from_cache: false,
        };
        self.dir.put_capture(StoredCapture {
            receipt: receipt.clone(),
            etag: response.header("etag").map(str::to_owned),
            last_modified: response.header("last-modified").map(str::to_owned),
        })?;
        self.dir.point_url(&plan.url, &raw_id)?;
        Ok(receipt)
    }

    fn check_robots(
        &self,
        plan: &FetchPlan,
        url: &Url,
        requests: &mut usize,
        out: &mut Vec<RobotsReceipt>,
    ) -> Result<(), FetchError> {
        let o = origin(url);
        let cached = self.dir.robots(&o)?;
        let stored = if let Some(v) = cached.filter(|v| v.expires_at > plan.now_secs) {
            v
        } else {
            *requests += 1;
            if *requests > plan.max_requests {
                return Err(FetchError::InvalidPlan("request budget exceeded".into()));
            }
            let robots_url = format!("{o}/robots.txt");
            match self.fetch_robots(plan, &robots_url, requests) {
                Ok(r) if (400..500).contains(&r.status) => StoredRobots {
                    body: vec![],
                    status: Some(r.status),
                    decision: "unavailable-allowed".into(),
                    expires_at: plan.now_secs + ROBOTS_TTL_SECS,
                },
                Ok(r) if r.status >= 500 => StoredRobots {
                    body: r.body,
                    status: Some(r.status),
                    decision: "temporarily-denied".into(),
                    expires_at: plan.now_secs + ROBOTS_TTL_SECS,
                },
                Ok(r) if (300..400).contains(&r.status) => StoredRobots {
                    body: r.body,
                    status: Some(r.status),
                    decision: "redirect-denied".into(),
                    expires_at: plan.now_secs + ROBOTS_TTL_SECS,
                },
                Ok(r) => {
                    let doc = sim_codec_robots::parse_robots(&r.body, &Default::default(), None)
                        .map_err(|e| FetchError::Decode(e.to_string()))?;
                    let decision = if doc.allows(&plan.user_agent, url.path()) {
                        "allowed"
                    } else {
                        "denied"
                    };
                    StoredRobots {
                        body: r.body,
                        status: Some(r.status),
                        decision: decision.into(),
                        expires_at: plan.now_secs + ROBOTS_TTL_SECS,
                    }
                }
                Err(_) => StoredRobots {
                    body: vec![],
                    status: None,
                    decision: "unreachable-denied".into(),
                    expires_at: plan.now_secs + ROBOTS_TTL_SECS,
                },
            }
        };
        self.dir.put_robots(&o, stored.clone())?;
        out.push(RobotsReceipt {
            origin: o,
            status: stored.status,
            decision: stored.decision.clone(),
            expires_at: stored.expires_at,
        });
        if stored.decision.contains("denied") {
            return Err(FetchError::RobotsDenied(stored.decision));
        }
        Ok(())
    }

    fn fetch_robots(
        &self,
        plan: &FetchPlan,
        initial_url: &str,
        requests: &mut usize,
    ) -> Result<HttpResponse, FetchError> {
        // RFC 9309 permits following robots redirects, but they remain ordinary
        // egress effects: each hop is independently authorized and budgeted.
        let mut current = initial_url.to_owned();
        for hop in 0..=plan.max_redirects.min(5) {
            let parsed = Url::parse(&current).map_err(net)?;
            self.egress.authorize("GET", &parsed)?;
            let response = self.transport.execute(
                "GET",
                &current,
                &[],
                sim_codec_robots::RFC_MINIMUM_BYTES,
            )?;
            if !(300..400).contains(&response.status) {
                return Ok(response);
            }
            if hop == plan.max_redirects.min(5) {
                return Ok(response);
            }
            *requests += 1;
            if *requests > plan.max_requests {
                return Err(FetchError::InvalidPlan("request budget exceeded".into()));
            }
            let location = response
                .header("location")
                .ok_or_else(|| FetchError::Transport("robots redirect without location".into()))?;
            current = resolve_location(&parsed, location)?;
        }
        unreachable!("bounded robots redirect loop always returns")
    }
}

fn require_net_http(cx: &Cx) -> Result<(), FetchError> {
    let names = ["net/http", "network.http", "network", "http", "web-fetch"];
    if names
        .iter()
        .any(|n| cx.require(&CapabilityName::new(*n)).is_ok())
    {
        Ok(())
    } else {
        Err(FetchError::Capability(
            "missing net/http capability (or a compatibility alias)".into(),
        ))
    }
}
fn validate_plan(p: &FetchPlan) -> Result<(), FetchError> {
    if p.method != "GET" && p.method != "HEAD" {
        return Err(FetchError::InvalidPlan(
            "only safe GET/HEAD capture is admitted".into(),
        ));
    }
    if p.max_body_bytes == 0
        || p.max_requests == 0
        || p.max_origins == 0
        || p.max_requests_per_origin == 0
    {
        return Err(FetchError::InvalidPlan("budgets must be non-zero".into()));
    }
    Url::parse(&p.url).map_err(net)?;
    Ok(())
}
fn origin(u: &Url) -> String {
    let default =
        (u.scheme() == "http" && u.port() == 80) || (u.scheme() == "https" && u.port() == 443);
    if default {
        format!("{}://{}", u.scheme(), u.host())
    } else {
        format!("{}://{}:{}", u.scheme(), u.host(), u.port())
    }
}
fn resolve_location(base: &Url, next: &str) -> Result<String, FetchError> {
    if next.starts_with("http://") || next.starts_with("https://") {
        return Url::parse(next).map(|u| u.as_str().to_owned()).map_err(net);
    }
    if !next.starts_with('/') {
        return Err(FetchError::PolicyDenied(
            "relative redirect must be origin-absolute".into(),
        ));
    }
    Ok(format!("{}{}", origin(base), next))
}
fn media_type(v: Option<&str>) -> Option<String> {
    v.map(|s| s.split(';').next().unwrap_or(s).trim().to_ascii_lowercase())
}
