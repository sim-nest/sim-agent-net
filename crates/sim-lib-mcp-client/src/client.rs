use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Map, Value, json};
use sim_cancel::Cancellation;
use sim_kernel::{Cx, Symbol};
use sim_lib_skill::{SkillCard, SkillPolicy, SkillRole};
use sim_shape::{AnyShape, shape_value};

use crate::{
    BindingError, BindingPeer, CacheDisposition, CacheKey, CallContext, ClientCache, ClientError,
    ClientEvent, ClientLedger, Discovery, EndpointIdentity, Era, InputBroker, InputRequest,
    Invocation, Outcome, PeerReply, SchemaContract, Subscription,
};

/// Resource bounds and local policy for one client.
#[derive(Clone, Debug)]
pub struct ClientPolicy {
    /// Maximum discovery lifetime even when the server advertises longer.
    pub discovery_ttl: Duration,
    /// Maximum result lifetime even when `ttlMs` advertises longer.
    pub result_ttl: Duration,
    /// Maximum MRTR rounds.
    pub maximum_input_rounds: usize,
    /// Maximum aggregate encoded input bytes.
    pub maximum_input_bytes: usize,
    /// Maximum total input values.
    pub maximum_inputs: usize,
    /// Maximum imported icon descriptors.
    pub maximum_icons: usize,
    /// Maximum bytes across icon URI and media-type metadata.
    pub maximum_icon_metadata_bytes: usize,
}

impl Default for ClientPolicy {
    fn default() -> Self {
        Self {
            discovery_ttl: Duration::from_secs(300),
            result_ttl: Duration::from_secs(60),
            maximum_input_rounds: 3,
            maximum_input_bytes: 64 * 1024,
            maximum_inputs: 16,
            maximum_icons: 8,
            maximum_icon_metadata_bytes: 4096,
        }
    }
}

struct EraEntry {
    discovery: Discovery,
    expires_at_ms: u64,
}

/// Modern-first client over one HTTP resource or child-process instance.
pub struct Client {
    peer: Arc<dyn BindingPeer>,
    policy: ClientPolicy,
    era: Mutex<Option<EraEntry>>,
    next_id: std::sync::atomic::AtomicU64,
    cache: Arc<dyn ClientCache>,
    broker: Arc<dyn InputBroker>,
    ledger: Arc<dyn ClientLedger>,
}

impl Client {
    /// Constructs a bounded client with injected cache, input broker, and ledger.
    pub fn new(
        peer: Arc<dyn BindingPeer>,
        policy: ClientPolicy,
        cache: Arc<dyn ClientCache>,
        broker: Arc<dyn InputBroker>,
        ledger: Arc<dyn ClientLedger>,
    ) -> Result<Self, ClientError> {
        if policy.discovery_ttl.is_zero()
            || policy.result_ttl.is_zero()
            || policy.maximum_input_rounds == 0
        {
            return Err(ClientError::Policy(
                "client time and MRTR bounds must be non-zero".into(),
            ));
        }
        Ok(Self {
            peer,
            policy,
            era: Mutex::new(None),
            next_id: std::sync::atomic::AtomicU64::new(1),
            cache,
            broker,
            ledger,
        })
    }

    /// Current endpoint identity, excluding credentials and display metadata.
    pub fn endpoint(&self) -> EndpointIdentity {
        self.peer.endpoint()
    }

    /// Explicitly invalidates discovery after process exit, material endpoint
    /// change, incompatible response, or host policy expiry.
    pub fn invalidate(&self) -> Result<(), ClientError> {
        *self
            .era
            .lock()
            .map_err(|_| ClientError::Policy("poisoned era cache".into()))? = None;
        self.cache.clear_endpoint(&self.endpoint())
    }

    /// Probes and validates discovery before any application operation.
    pub fn discover(&self, context: &CallContext<'_>) -> Result<Discovery, ClientError> {
        if context.cancellation.is_cancelled() {
            return Err(BindingError::Cancelled.into());
        }
        if let Some(entry) = self
            .era
            .lock()
            .map_err(|_| ClientError::Policy("poisoned era cache".into()))?
            .as_ref()
        {
            if context.now_ms < entry.expires_at_ms {
                return Ok(entry.discovery.clone());
            }
        }
        let endpoint = self.endpoint();
        self.ledger.record(&endpoint, "server/discover", "probe");
        let id = self.id();
        let modern = self.peer.request(
            Era::Modern,
            id,
            "server/discover",
            &json!({}),
            context.cancellation,
            context.deadline_ms,
        );
        let discovery = match modern {
            Ok(PeerReply::Complete(value)) => parse_discovery(Era::Modern, &value)?,
            Ok(_) => return self.incompatible("discovery was not complete"),
            Err(error) if recognized_legacy_probe(self.peer.binding_kind(), &error) => {
                self.ledger.record(&endpoint, "initialize", "legacy-probe");
                let reply = self.peer.request(
                    Era::Legacy,
                    self.id(),
                    "initialize",
                    &legacy_initialize(),
                    context.cancellation,
                    context.deadline_ms,
                )?;
                match reply {
                    PeerReply::Complete(value) => parse_discovery(Era::Legacy, &value)?,
                    _ => return self.incompatible("legacy initialize was not complete"),
                }
            }
            Err(BindingError::ProcessExited(code)) => {
                self.invalidate()?;
                return Err(BindingError::ProcessExited(code).into());
            }
            Err(error) => {
                return Err(if is_unrecognized_probe(&error) {
                    ClientError::UnrecognizedDiscovery
                } else {
                    error.into()
                });
            }
        };
        let ttl = discovery.ttl.min(self.policy.discovery_ttl);
        let expires_at_ms = context.now_ms.saturating_add(duration_ms(ttl));
        *self
            .era
            .lock()
            .map_err(|_| ClientError::Policy("poisoned era cache".into()))? = Some(EraEntry {
            discovery: discovery.clone(),
            expires_at_ms,
        });
        Ok(discovery)
    }

    /// Imports validated server cards without fetching icon content. Icon URI
    /// descriptors remain bounded inert metadata; no MCP or OAuth credential is
    /// ever forwarded to them.
    pub fn import_cards(
        &self,
        cx: &mut Cx,
        context: &CallContext<'_>,
    ) -> Result<Vec<McpCallable>, ClientError> {
        let discovery = self.discover(context)?;
        let reply = self.peer.request(
            discovery.era,
            self.id(),
            "server/cards",
            &json!({}),
            context.cancellation,
            context.deadline_ms,
        )?;
        let value = complete(reply)?;
        let cards = value
            .get("cards")
            .and_then(Value::as_array)
            .ok_or_else(|| ClientError::Schema("server cards must be an array".into()))?;
        cards
            .iter()
            .map(|value| self.import_card(cx, value))
            .collect()
    }

    /// Invokes one callable through schema, MRTR, cancellation, deadline,
    /// ledger, and scoped-cache policy.
    pub fn invoke(
        &self,
        call: &McpCallable,
        parameters: Value,
        context: &CallContext<'_>,
    ) -> Result<Outcome, ClientError> {
        call.invocation.input.validate(&parameters)?;
        let discovery = self.discover(context)?;
        let key = cache_key(
            self.endpoint(),
            &discovery,
            &call.invocation,
            &parameters,
            context,
        )?;
        let may_cache = call.invocation.cache_eligible && !call.invocation.effecting;
        if may_cache {
            if let CacheDisposition::Hit(value) = self.cache.get(&key, context.now_ms)? {
                call.invocation.output.validate(&value)?;
                self.ledger
                    .record(&self.endpoint(), &call.invocation.method, "cache-hit");
                return Ok(Outcome {
                    value,
                    ttl_ms: None,
                });
            }
        }
        self.ledger
            .record(&self.endpoint(), &call.invocation.method, "call");
        let mut params = parameters;
        let mut rounds = 0usize;
        let mut aggregate_bytes = 0usize;
        let mut aggregate_inputs = 0usize;
        loop {
            if context.cancellation.is_cancelled() {
                return Err(BindingError::Cancelled.into());
            }
            let reply = match self.peer.request(
                discovery.era,
                self.id(),
                &call.invocation.method,
                &params,
                context.cancellation,
                context.deadline_ms,
            ) {
                Err(BindingError::Incompatible(message)) => {
                    self.invalidate()?;
                    return Err(ClientError::Binding(BindingError::Incompatible(message)));
                }
                Err(BindingError::ProcessExited(code)) => {
                    self.invalidate()?;
                    return Err(BindingError::ProcessExited(code).into());
                }
                other => other?,
            };
            match reply {
                PeerReply::Complete(value) => {
                    let (value, ttl_ms) = unwrap_complete(value)?;
                    call.invocation.output.validate(&value)?;
                    if may_cache {
                        let ttl = ttl_ms
                            .unwrap_or(duration_ms(self.policy.result_ttl))
                            .min(duration_ms(self.policy.result_ttl));
                        if ttl > 0 {
                            self.cache.put(
                                key,
                                value.clone(),
                                context.now_ms.saturating_add(ttl),
                            )?;
                        }
                    }
                    self.ledger
                        .record(&self.endpoint(), &call.invocation.method, "complete");
                    return Ok(Outcome { value, ttl_ms });
                }
                PeerReply::InputRequired {
                    request_state,
                    requested,
                } => {
                    rounds += 1;
                    if rounds > self.policy.maximum_input_rounds || requested.is_empty() {
                        return Err(ClientError::InputRequired(
                            "round limit or empty request".into(),
                        ));
                    }
                    if requested
                        .values()
                        .any(|cap| !context.input_capabilities.contains(cap))
                    {
                        return Err(ClientError::InputRequired(
                            "undeclared input capability".into(),
                        ));
                    }
                    let remaining_ms = context.deadline_ms.saturating_sub(context.now_ms);
                    if remaining_ms == 0 {
                        return Err(BindingError::Timeout.into());
                    }
                    let inputs = self.broker.acquire(InputRequest {
                        requested: &requested,
                        request_state: &request_state,
                        remaining_ms,
                    })?;
                    aggregate_inputs = aggregate_inputs.saturating_add(inputs.len());
                    aggregate_bytes = aggregate_bytes.saturating_add(
                        serde_json::to_vec(&inputs)
                            .map_err(|e| ClientError::InputRequired(e.to_string()))?
                            .len(),
                    );
                    if aggregate_inputs > self.policy.maximum_inputs
                        || aggregate_bytes > self.policy.maximum_input_bytes
                    {
                        return Err(ClientError::InputRequired(
                            "aggregate input bound exceeded".into(),
                        ));
                    }
                    params = json!({"requestState": request_state, "inputs": inputs});
                    self.ledger
                        .record(&self.endpoint(), &call.invocation.method, "input-retry");
                }
                PeerReply::Stream(_) => {
                    return self.incompatible("ordinary invocation returned a subscription stream");
                }
            }
        }
    }

    /// Opens and validates one backpressured subscription sequence. Each call
    /// has independent cancellation and can run concurrently with other calls.
    pub fn subscribe(
        &self,
        method: &str,
        parameters: Value,
        context: &CallContext<'_>,
    ) -> Result<Subscription, ClientError> {
        let discovery = self.discover(context)?;
        let response = self.peer.request(
            discovery.era,
            self.id(),
            method,
            &parameters,
            context.cancellation,
            context.deadline_ms,
        );
        let frames = match response {
            Err(BindingError::ProcessExited(code)) => {
                self.invalidate()?;
                return Err(BindingError::ProcessExited(code).into());
            }
            Err(BindingError::Incompatible(message)) => {
                self.invalidate()?;
                return Err(BindingError::Incompatible(message).into());
            }
            Err(error) => return Err(error.into()),
            Ok(PeerReply::Stream(frames)) => frames,
            Ok(_) => {
                return Err(ClientError::Subscription(
                    "subscription did not return a stream".into(),
                ));
            }
        };
        validate_subscription(frames, context.cancellation)
    }

    fn import_card(&self, _cx: &mut Cx, value: &Value) -> Result<McpCallable, ClientError> {
        let object = value
            .as_object()
            .ok_or_else(|| ClientError::Schema("Card must be an object".into()))?;
        validate_keys(
            object,
            &[
                "name",
                "title",
                "description",
                "role",
                "inputSchema",
                "outputSchema",
                "cacheEligible",
                "effecting",
                "icons",
            ],
        )?;
        let name = string(object, "name")?;
        let title = string(object, "title")?;
        let description = string(object, "description")?;
        let role = match string(object, "role")?.as_str() {
            "tool" => SkillRole::Tool,
            "resource" => SkillRole::Resource,
            "prompt" => SkillRole::Prompt,
            _ => return Err(ClientError::Schema("unsupported Card role".into())),
        };
        let icons = validate_icons(object.get("icons"), &self.policy)?;
        let input = SchemaContract::new(
            object
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({})),
            64 * 1024,
            32,
        )?;
        let output = SchemaContract::new(
            object
                .get("outputSchema")
                .cloned()
                .unwrap_or_else(|| json!({})),
            64 * 1024,
            32,
        )?;
        let id = format!("mcp.{}", stable_name(&name)?);
        let invocation = Invocation {
            method: match role {
                SkillRole::Tool => "tools/call",
                SkillRole::Resource => "resources/read",
                _ => "prompts/get",
            }
            .into(),
            operation: name.clone(),
            input,
            output,
            cache_eligible: object
                .get("cacheEligible")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            effecting: object
                .get("effecting")
                .and_then(Value::as_bool)
                .unwrap_or(role == SkillRole::Tool),
        };
        let card = SkillCard {
            id: id.clone(),
            symbol: Symbol::qualified("skill", id.clone()),
            aliases: Vec::new(),
            origin: Symbol::new("mcp-client"),
            title,
            description,
            input_shape: shape_value(
                Symbol::qualified("mcp-client", format!("{id}-input")),
                Arc::new(AnyShape),
            ),
            output_shape: shape_value(
                Symbol::qualified("mcp-client", format!("{id}-output")),
                Arc::new(AnyShape),
            ),
            roles: vec![role],
            capabilities: vec![sim_lib_skill::skill_specific_call_capability(&id)],
            policy: SkillPolicy::default(),
            transport_id: self.endpoint().to_string(),
            transport_kind: "mcp-client".into(),
            operation: name,
        };
        Ok(McpCallable {
            card,
            invocation,
            icons,
        })
    }

    fn id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
    fn incompatible<T>(&self, message: &str) -> Result<T, ClientError> {
        self.invalidate()?;
        Err(ClientError::Binding(BindingError::Incompatible(
            message.into(),
        )))
    }
}

/// The single imported runtime callable path.
#[derive(Clone)]
pub struct McpCallable {
    card: SkillCard,
    invocation: Invocation,
    icons: Vec<IconDescriptor>,
}
impl McpCallable {
    /// Canonical runtime Card/SkillCard projection.
    pub fn card(&self) -> &SkillCard {
        &self.card
    }
    /// Validated invocation metadata.
    pub fn invocation(&self) -> &Invocation {
        &self.invocation
    }
    /// Validated inert icon descriptors. The client never dereferences them.
    pub fn icons(&self) -> &[IconDescriptor] {
        &self.icons
    }
}

/// Bounded inert icon metadata imported from a Card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconDescriptor {
    /// Absolute or relative descriptor URI retained verbatim without fetching.
    pub src: String,
    /// Declared media type, when present.
    pub media_type: Option<String>,
}

fn recognized_legacy_probe(kind: &str, error: &BindingError) -> bool {
    match (kind, error) {
        ("stdio", BindingError::Rpc { code: -32601, .. }) => true,
        ("http", BindingError::Http { status: 400, body }) => exact_method_not_found(body),
        _ => false,
    }
}
fn exact_method_not_found(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|value| {
            value.pointer("/error/code").and_then(Value::as_i64) == Some(-32601)
                && value.pointer("/error/message").and_then(Value::as_str)
                    == Some("Method not found")
        })
}
fn is_unrecognized_probe(error: &BindingError) -> bool {
    matches!(error, BindingError::Rpc { .. } | BindingError::Http { .. })
}
fn legacy_initialize() -> Value {
    json!({"protocolVersion":"2025-03-26","clientInfo":{"name":"sim-lib-mcp-client","version":env!("CARGO_PKG_VERSION")},"capabilities":{}})
}

fn parse_discovery(era: Era, value: &Value) -> Result<Discovery, ClientError> {
    let object = value
        .as_object()
        .ok_or(ClientError::UnrecognizedDiscovery)?;
    let (version, extensions, info, ttl_ms) = if era == Era::Modern {
        if object.get("resultType").and_then(Value::as_str) != Some("complete") {
            return Err(ClientError::UnrecognizedDiscovery);
        }
        let versions = object
            .get("supportedVersions")
            .and_then(Value::as_array)
            .ok_or(ClientError::UnrecognizedDiscovery)?;
        let version = versions
            .first()
            .and_then(Value::as_str)
            .ok_or(ClientError::UnrecognizedDiscovery)?
            .to_owned();
        let extensions = strings(object.get("extensions"))?;
        (
            version,
            extensions,
            object.get("serverInfo"),
            object.get("ttlMs").and_then(Value::as_u64).or_else(|| {
                object
                    .get("ttl")
                    .and_then(Value::as_u64)
                    .map(|v| v.saturating_mul(1000))
            }),
        )
    } else {
        (
            string(object, "protocolVersion")?,
            strings(object.get("extensions")).unwrap_or_default(),
            object.get("serverInfo"),
            Some(60_000),
        )
    };
    if version.is_empty() {
        return Err(ClientError::Schema("empty protocol version".into()));
    }
    let mut unique = BTreeSet::new();
    if extensions
        .iter()
        .any(|value| value.is_empty() || !unique.insert(value))
    {
        return Err(ClientError::Schema("invalid duplicate extension".into()));
    }
    let info = info
        .and_then(Value::as_object)
        .ok_or_else(|| ClientError::Schema("missing serverInfo".into()))?;
    Ok(Discovery {
        era,
        version,
        extensions,
        server_name: string(info, "name")?,
        server_version: string(info, "version")?,
        ttl: Duration::from_millis(ttl_ms.unwrap_or(60_000)),
    })
}

fn cache_key(
    endpoint: EndpointIdentity,
    discovery: &Discovery,
    invocation: &Invocation,
    parameters: &Value,
    context: &CallContext<'_>,
) -> Result<CacheKey, ClientError> {
    Ok(CacheKey {
        endpoint,
        era: discovery.era,
        version: discovery.version.clone(),
        principal_scope: context.principal_scope.to_owned(),
        extensions: discovery.extensions.clone(),
        method: invocation.method.clone(),
        canonical_parameters: serde_json::to_string(&canonical(parameters))
            .map_err(|e| ClientError::Cache(e.to_string()))?,
        pagination_cursor: context.pagination_cursor.map(str::to_owned),
    })
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        other => other.clone(),
    }
}
fn unwrap_complete(value: Value) -> Result<(Value, Option<u64>), ClientError> {
    if value
        .get("resultType")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "complete")
    {
        return Err(ClientError::Policy(
            "only complete outcomes may publish".into(),
        ));
    }
    let ttl = value.get("ttlMs").and_then(Value::as_u64);
    Ok((value.get("value").cloned().unwrap_or(value), ttl))
}
fn complete(reply: PeerReply) -> Result<Value, ClientError> {
    match reply {
        PeerReply::Complete(value) => Ok(value),
        _ => Err(ClientError::Schema("expected complete response".into())),
    }
}
fn strings(value: Option<&Value>) -> Result<Vec<String>, ClientError> {
    value.map_or(Ok(Vec::new()), |value| {
        value
            .as_array()
            .ok_or_else(|| ClientError::Schema("expected string array".into()))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ClientError::Schema("expected string array".into()))
            })
            .collect()
    })
}
fn string(object: &Map<String, Value>, name: &str) -> Result<String, ClientError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ClientError::Schema(format!("missing string {name}")))
}
fn validate_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ClientError> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Err(ClientError::Schema(format!("unknown Card field {key}")))
    } else {
        Ok(())
    }
}
fn validate_icons(
    value: Option<&Value>,
    policy: &ClientPolicy,
) -> Result<Vec<IconDescriptor>, ClientError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let icons = value
        .as_array()
        .ok_or_else(|| ClientError::Schema("icons must be an array".into()))?;
    if icons.len() > policy.maximum_icons {
        return Err(ClientError::Schema("icon descriptor count exceeded".into()));
    }
    let bytes: usize = icons
        .iter()
        .map(|icon| {
            icon.get("src").and_then(Value::as_str).map_or(0, str::len)
                + icon
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .map_or(0, str::len)
        })
        .sum();
    if bytes > policy.maximum_icon_metadata_bytes
        || icons
            .iter()
            .any(|icon| icon.get("src").and_then(Value::as_str).is_none())
    {
        return Err(ClientError::Schema(
            "invalid or oversized icon metadata".into(),
        ));
    }
    icons
        .iter()
        .map(|icon| {
            let object = icon
                .as_object()
                .ok_or_else(|| ClientError::Schema("icon descriptor must be an object".into()))?;
            validate_keys(object, &["src", "mediaType"])?;
            Ok(IconDescriptor {
                src: string(object, "src")?,
                media_type: object
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}
fn stable_name(value: &str) -> Result<String, ClientError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Err(ClientError::Schema("unsafe callable name".into()))
    } else {
        Ok(value.to_owned())
    }
}
fn duration_ms(value: Duration) -> u64 {
    u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
}

fn validate_subscription(
    frames: Vec<Value>,
    cancellation: &Cancellation,
) -> Result<Subscription, ClientError> {
    let first = frames
        .first()
        .and_then(Value::as_object)
        .ok_or_else(|| ClientError::Subscription("missing acknowledgement".into()))?;
    if first.get("type").and_then(Value::as_str) != Some("acknowledged") {
        return Err(ClientError::Subscription(
            "first frame must acknowledge".into(),
        ));
    }
    let id = first
        .get("subscriptionId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientError::Subscription("acknowledgement missing id".into()))?
        .to_owned();
    let mut events = vec![ClientEvent::Acknowledged(id.clone())];
    for (index, frame) in frames.iter().enumerate().skip(1) {
        let object = frame
            .as_object()
            .ok_or_else(|| ClientError::Subscription("stream frame must be an object".into()))?;
        if object.get("subscriptionId").and_then(Value::as_str) != Some(&id) {
            return Err(ClientError::Subscription("subscription id mismatch".into()));
        }
        match object.get("type").and_then(Value::as_str) {
            Some("event") if index + 1 < frames.len() => events.push(ClientEvent::Event(
                object
                    .get("event")
                    .cloned()
                    .ok_or_else(|| ClientError::Subscription("event body missing".into()))?,
            )),
            Some("complete") if index + 1 == frames.len() => events.push(ClientEvent::Complete {
                completed_at_ms: object
                    .get("completedAtMs")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ClientError::Subscription("dated terminal required".into()))?,
                cancelled: object
                    .get("cancelled")
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| cancellation.is_cancelled()),
            }),
            _ => {
                return Err(ClientError::Subscription(
                    "invalid acknowledgement/event/terminal sequence".into(),
                ));
            }
        }
    }
    if !matches!(events.last(), Some(ClientEvent::Complete { .. })) {
        return Err(ClientError::Subscription("dated terminal required".into()));
    }
    Ok(Subscription { id, events })
}
