use super::*;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct FixtureCodec;
impl SearchWireCodec for FixtureCodec {
    fn codec_id(&self) -> &str {
        "fixture"
    }
    fn codec_version(&self) -> &str {
        "1"
    }
    fn encode_request(&self, q: &SearchQuery, _: DecodeLimits) -> Result<Vec<u8>, SearchError> {
        Ok(q.text.as_bytes().to_vec())
    }
    fn decode_config(&self, input: &[u8], _: DecodeLimits) -> Result<Datum, SearchError> {
        Ok(Datum::Bytes(input.to_vec()))
    }
    fn decode_response(
        &self,
        input: &[u8],
        q: &SearchQuery,
        _: DecodeLimits,
    ) -> Result<SearchPage, SearchError> {
        let text =
            String::from_utf8(input.to_vec()).map_err(|e| SearchError::Wire(e.to_string()))?;
        let mut parts = text.split('|');
        let uri = parts.next().unwrap_or_default();
        let continuation = parts.next().filter(|v| !v.is_empty()).map(str::to_owned);
        let observations = if uri.starts_with("bad") {
            vec![]
        } else {
            vec![SearchObservation::checked(
                uri,
                Some(ProviderClaim {
                    provider: "fixture-provider".into(),
                    uri: uri.into(),
                    title: Some("row".into()),
                    snippet: None,
                    position: Some(1),
                }),
                None,
            )?]
        };
        Ok(SearchPage {
            query: q.clone(),
            observations,
            continuation,
        })
    }
}
struct Secrets;
impl SecretResolver for Secrets {
    fn principal_headers(
        &self,
        _: &PrincipalRef,
    ) -> Result<Vec<(String, String)>, SearchHttpError> {
        Ok(vec![("authorization".into(), "TOP-SECRET".into())])
    }
}
struct Script {
    replies: Mutex<VecDeque<HttpResponse>>,
    calls: AtomicUsize,
    saw_secret: Mutex<bool>,
}
impl SearchHttpClient for Script {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, SearchHttpError> {
        *self.saw_secret.lock().unwrap() = request.headers.iter().any(|(_, v)| v == "TOP-SECRET");
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| SearchHttpError::Transport("empty script".into()))
    }
}
fn config(minimum_interval: Duration, response_bytes: usize) -> SearchSiteConfig {
    SearchSiteConfig {
        site_id: "fixture-site".into(),
        endpoint: "https://invalid.test/search".into(),
        codec_id: "fixture".into(),
        config_revision: "r7".into(),
        principal: PrincipalRef("vault/search/site".into()),
        limits: SiteLimits {
            concurrent_requests: 1,
            minimum_interval,
            timeout: Duration::from_secs(1),
            response_bytes,
            pages: 2,
            egress_bytes: 1024,
            query_timeout: Duration::from_secs(2),
        },
        observation_ttl: Duration::from_secs(60),
    }
}
fn query() -> SearchQuery {
    SearchQuery::checked("needle".into(), vec![], None, 10).unwrap()
}

#[test]
fn live_record_and_cassette_replay_are_byte_identical_and_redacted() {
    let http = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            HttpResponse {
                status: 200,
                headers: vec![],
                body: b"https://one.test|next".to_vec(),
            },
            HttpResponse {
                status: 200,
                headers: vec![],
                body: b"https://two.test|".to_vec(),
            },
        ])),
        calls: AtomicUsize::new(0),
        saw_secret: Mutex::new(false),
    });
    let transport = HttpSearchTransport::new(
        "fixture-http",
        FixtureCodec,
        config(Duration::ZERO, 1000),
        http.clone(),
        Arc::new(Secrets),
        DecodeLimits::default(),
    )
    .unwrap();
    let live = transport.search(query(), CallMode::Record).unwrap();
    let replay = transport.search(query(), CallMode::Replay).unwrap();
    assert_eq!(live, replay);
    assert_eq!(live.pages.len(), 2);
    assert_eq!(http.calls.load(Ordering::SeqCst), 2);
    assert!(*http.saw_secret.lock().unwrap());
    let durable = format!(
        "{live:?}{:?}{:?}",
        transport.config,
        transport.call_key(&query())
    );
    assert!(!durable.contains("TOP-SECRET"));
}

#[test]
fn discovery_is_explicit_and_cached_without_http() {
    let http = Arc::new(Script {
        replies: Mutex::new(VecDeque::new()),
        calls: AtomicUsize::new(0),
        saw_secret: Mutex::new(false),
    });
    let transport = HttpSearchTransport::new(
        "fixture-http",
        FixtureCodec,
        config(Duration::ZERO, 1000),
        http.clone(),
        Arc::new(Secrets),
        DecodeLimits::default(),
    )
    .unwrap();
    let now = Instant::now();
    assert_eq!(
        transport.discover_config(now).unwrap(),
        transport.discover_config(now).unwrap()
    );
    assert_eq!(http.calls.load(Ordering::SeqCst), 0);
    let card = transport.card();
    assert_eq!(card.roles, vec![SkillRole::Retriever, SkillRole::Tool]);
    assert!(card.capabilities.contains(&CapabilityName::new("net/http")));
    assert_eq!(card.transport_kind, "search-http");
}

#[test]
fn limits_throttle_and_reject_oversize_before_decode() {
    let http = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([HttpResponse {
            status: 200,
            headers: vec![],
            body: vec![b'x'; 9],
        }])),
        calls: AtomicUsize::new(0),
        saw_secret: Mutex::new(false),
    });
    let transport = HttpSearchTransport::new(
        "fixture-http",
        FixtureCodec,
        config(Duration::from_secs(60), 8),
        http,
        Arc::new(Secrets),
        DecodeLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        transport.search(query(), CallMode::Live),
        Err(SearchHttpError::Policy(_))
    ));
}

#[test]
fn registry_uses_only_the_object_safe_skill_transport() {
    fn accepts(_: Arc<dyn SkillTransport>) {}
    let http = Arc::new(Script {
        replies: Mutex::new(VecDeque::new()),
        calls: AtomicUsize::new(0),
        saw_secret: Mutex::new(false),
    });
    accepts(Arc::new(
        HttpSearchTransport::new(
            "fixture-http",
            FixtureCodec,
            config(Duration::ZERO, 1000),
            http,
            Arc::new(Secrets),
            DecodeLimits::default(),
        )
        .unwrap(),
    ));
    let registry = include_str!("../../sim-lib-skill/src/registry.rs");
    assert!(!registry.contains("HttpSearchTransport"));
    assert!(!registry.contains("SearchWireCodec"));
}
