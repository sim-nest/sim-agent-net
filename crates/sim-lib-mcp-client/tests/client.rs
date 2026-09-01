use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use sim_cancel::Cancellation;
use sim_kernel::testing::bare_cx;
use sim_lib_mcp_client::*;

#[derive(Clone, Debug)]
struct Seen {
    era: Era,
    id: u64,
    method: String,
    params: Value,
}

struct Peer {
    kind: &'static str,
    endpoint: EndpointIdentity,
    replies: Mutex<VecDeque<Result<PeerReply, BindingError>>>,
    seen: Mutex<Vec<Seen>>,
}
impl Peer {
    fn new(kind: &'static str, replies: Vec<Result<PeerReply, BindingError>>) -> Self {
        Self {
            kind,
            endpoint: if kind == "http" {
                EndpointIdentity::http("HTTP://LOCALHOST:80/mcp/?token=ignored").unwrap()
            } else {
                EndpointIdentity::process("child-1").unwrap()
            },
            replies: Mutex::new(replies.into()),
            seen: Mutex::new(Vec::new()),
        }
    }
    fn methods(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|row| row.method.clone())
            .collect()
    }
}
impl BindingPeer for Peer {
    fn endpoint(&self) -> EndpointIdentity {
        self.endpoint.clone()
    }
    fn binding_kind(&self) -> &'static str {
        self.kind
    }
    fn request(
        &self,
        era: Era,
        id: u64,
        method: &str,
        params: &Value,
        _: &Cancellation,
        _: u64,
    ) -> Result<PeerReply, BindingError> {
        self.seen.lock().unwrap().push(Seen {
            era,
            id,
            method: method.into(),
            params: params.clone(),
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("fixture reply")
    }
}

struct Broker;
impl InputBroker for Broker {
    fn acquire(&self, request: InputRequest<'_>) -> Result<BTreeMap<String, Value>, ClientError> {
        assert_eq!(request.request_state, &json!({"opaque":"exact"}));
        Ok(request
            .requested
            .keys()
            .map(|name| (name.clone(), json!(true)))
            .collect())
    }
}
#[derive(Default)]
struct Ledger(Mutex<Vec<String>>);
impl ClientLedger for Ledger {
    fn record(&self, _: &EndpointIdentity, method: &str, phase: &str) {
        self.0.lock().unwrap().push(format!("{method}:{phase}"));
    }
}

fn discovery() -> PeerReply {
    PeerReply::Complete(
        json!({"resultType":"complete","supportedVersions":["2026-07-28"],"extensions":["cards","subscriptions"],"serverInfo":{"name":"fixture","version":"1"},"ttlMs":1000}),
    )
}
fn legacy_discovery() -> PeerReply {
    PeerReply::Complete(
        json!({"protocolVersion":"2025-03-26","extensions":[],"serverInfo":{"name":"legacy","version":"1"}}),
    )
}
fn cards(cache: bool, effecting: bool) -> PeerReply {
    PeerReply::Complete(
        json!({"cards":[{"name":"echo","title":"Echo","description":"one callable","role":"tool","inputSchema":{"type":"object","required":["text"],"properties":{"text":{"type":"string"}}},"outputSchema":{"type":"string"},"cacheEligible":cache,"effecting":effecting,"icons":[{"src":"https://icons.invalid/echo.svg","mediaType":"image/svg+xml"}]}]}),
    )
}
fn context<'a>(
    principal: &'a str,
    cancellation: &'a Cancellation,
    caps: &'a [String],
    now: u64,
) -> CallContext<'a> {
    CallContext {
        principal_scope: principal,
        input_capabilities: caps,
        pagination_cursor: Some("cursor-1"),
        deadline_ms: now + 10_000,
        now_ms: now,
        cancellation,
    }
}
fn make_client(peer: Arc<Peer>, capacity: usize) -> Client {
    Client::new(
        peer,
        ClientPolicy::default(),
        Arc::new(MemoryLruCache::new(capacity).unwrap()),
        Arc::new(Broker),
        Arc::new(Ledger::default()),
    )
    .unwrap()
}

include!("client/connection.rs");
include!("client/cache.rs");
