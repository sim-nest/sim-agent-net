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
