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

#[test]
fn endpoint_identity_normalizes_and_excludes_credentials() {
    assert_eq!(
        EndpointIdentity::http("HTTPS://Example.COM:443//mcp/?display=x#y")
            .unwrap()
            .to_string(),
        "https://example.com/mcp"
    );
    assert!(EndpointIdentity::http("https://secret@example.com/mcp").is_err());
    assert_ne!(
        EndpointIdentity::process("child-1").unwrap(),
        EndpointIdentity::process("child-2").unwrap()
    );
}

#[test]
fn modern_probe_import_and_effecting_call_happen_once() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![
            Ok(discovery()),
            Ok(cards(false, true)),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"ok"}),
            )),
        ],
    ));
    let client = make_client(peer.clone(), 4);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("alice", &cancellation, &[], 1))
        .unwrap()
        .remove(0);
    assert_eq!(callable.icons()[0].src, "https://icons.invalid/echo.svg");
    let outcome = client
        .invoke(
            &callable,
            json!({"text":"hello"}),
            &context("alice", &cancellation, &[], 2),
        )
        .unwrap();
    assert_eq!(outcome.value, json!("ok"));
    assert_eq!(
        peer.methods(),
        ["server/discover", "server/cards", "tools/call"]
    );
    assert_eq!(peer.seen.lock().unwrap()[2].era, Era::Modern);
}

#[test]
fn exact_http_400_and_stdio_method_not_found_fall_back_before_call() {
    let body =
        serde_json::to_vec(&json!({"error":{"code":-32601,"message":"Method not found"}})).unwrap();
    for (kind, error) in [
        ("http", BindingError::Http { status: 400, body }),
        (
            "stdio",
            BindingError::Rpc {
                code: -32601,
                message: "anything".into(),
                data: None,
            },
        ),
    ] {
        let peer = Arc::new(Peer::new(
            kind,
            vec![
                Err(error),
                Ok(legacy_discovery()),
                Ok(cards(false, true)),
                Ok(PeerReply::Complete(json!({"value":"once"}))),
            ],
        ));
        let client = make_client(peer.clone(), 2);
        let cancellation = Cancellation::new();
        let mut cx = bare_cx();
        let callable = client
            .import_cards(&mut cx, &context("p", &cancellation, &[], 0))
            .unwrap()
            .remove(0);
        client
            .invoke(
                &callable,
                json!({"text":"x"}),
                &context("p", &cancellation, &[], 1),
            )
            .unwrap();
        assert_eq!(
            peer.methods(),
            [
                "server/discover",
                "initialize",
                "server/cards",
                "tools/call"
            ]
        );
        let seen = peer.seen.lock().unwrap();
        assert_ne!(seen[0].id, seen[1].id);
        assert_eq!(seen[3].era, Era::Legacy);
    }
}

#[test]
fn unrecognized_discovery_timeout_and_near_miss_http_body_fail_closed() {
    let near =
        serde_json::to_vec(&json!({"error":{"code":-32601,"message":"method not found"}})).unwrap();
    for error in [
        BindingError::Rpc {
            code: -32000,
            message: "no".into(),
            data: None,
        },
        BindingError::Http {
            status: 400,
            body: near,
        },
    ] {
        let peer = Arc::new(Peer::new("http", vec![Err(error)]));
        let client = make_client(peer.clone(), 1);
        let cancellation = Cancellation::new();
        assert!(matches!(
            client.discover(&context("p", &cancellation, &[], 0)),
            Err(ClientError::UnrecognizedDiscovery)
        ));
        assert_eq!(peer.methods(), ["server/discover"]);
    }
    let peer = Arc::new(Peer::new("stdio", vec![Err(BindingError::Timeout)]));
    let client = make_client(peer, 1);
    let cancellation = Cancellation::new();
    assert!(matches!(
        client.discover(&context("p", &cancellation, &[], 0)),
        Err(ClientError::Binding(BindingError::Timeout))
    ));
}

#[test]
fn schema_rejection_occurs_before_application_call() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(cards(false, true))],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("p", &cancellation, &[], 0))
        .unwrap()
        .remove(0);
    assert!(matches!(
        client.invoke(
            &callable,
            json!({"text":3}),
            &context("p", &cancellation, &[], 1)
        ),
        Err(ClientError::Schema(_))
    ));
    assert_eq!(peer.methods(), ["server/discover", "server/cards"]);
}

#[test]
fn mrtr_uses_fresh_id_exact_state_and_bounds_capabilities() {
    let requested = BTreeMap::from([("confirm".into(), "input.confirm".into())]);
    let peer = Arc::new(Peer::new(
        "stdio",
        vec![
            Ok(discovery()),
            Ok(cards(false, true)),
            Ok(PeerReply::InputRequired {
                request_state: json!({"opaque":"exact"}),
                requested,
            }),
            Ok(PeerReply::Complete(json!({"value":"done"}))),
        ],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    let caps = ["input.confirm".into()];
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("p", &cancellation, &caps, 0))
        .unwrap()
        .remove(0);
    client
        .invoke(
            &callable,
            json!({"text":"x"}),
            &context("p", &cancellation, &caps, 1),
        )
        .unwrap();
    let seen = peer.seen.lock().unwrap();
    assert_ne!(seen[2].id, seen[3].id);
    assert_eq!(seen[3].params["requestState"], json!({"opaque":"exact"}));
}

#[test]
fn complete_read_cache_preserves_scope_ttl_order_cursor_and_eviction() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![
            Ok(discovery()),
            Ok(cards(true, false)),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"a","ttlMs":10}),
            )),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"bob","ttlMs":10}),
            )),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"expired","ttlMs":10}),
            )),
        ],
    ));
    let client = make_client(peer.clone(), 2);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("alice", &cancellation, &[], 0))
        .unwrap()
        .remove(0);
    let params = json!({"text":"x","order":[3,1,2]});
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("alice", &cancellation, &[], 1)
            )
            .unwrap()
            .value,
        json!("a")
    );
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("alice", &cancellation, &[], 2)
            )
            .unwrap()
            .value,
        json!("a")
    );
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("bob", &cancellation, &[], 3)
            )
            .unwrap()
            .value,
        json!("bob")
    );
    assert_eq!(
        client
            .invoke(&callable, params, &context("alice", &cancellation, &[], 12))
            .unwrap()
            .value,
        json!("expired")
    );
    assert_eq!(
        peer.methods()
            .iter()
            .filter(|m| m.as_str() == "tools/call")
            .count(),
        3
    );
}

#[test]
fn subscription_checks_ids_terminal_and_cancellation() {
    let frames = vec![
        json!({"type":"acknowledged","subscriptionId":"s1"}),
        json!({"type":"event","subscriptionId":"s1","event":{"n":1}}),
        json!({"type":"complete","subscriptionId":"s1","completedAtMs":44,"cancelled":false}),
    ];
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(PeerReply::Stream(frames))],
    ));
    let client = make_client(peer, 1);
    let cancellation = Cancellation::new();
    let stream = client
        .subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 0),
        )
        .unwrap();
    assert_eq!(stream.id, "s1");
    assert!(matches!(
        stream.events.last(),
        Some(ClientEvent::Complete {
            completed_at_ms: 44,
            ..
        })
    ));

    let bad = vec![
        json!({"type":"acknowledged","subscriptionId":"a"}),
        json!({"type":"complete","subscriptionId":"b","completedAtMs":1}),
    ];
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(PeerReply::Stream(bad))],
    ));
    let client = make_client(peer, 1);
    assert!(matches!(
        client.subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 0)
        ),
        Err(ClientError::Subscription(_))
    ));
}

#[test]
fn concurrent_subscriptions_keep_independent_ids() {
    let stream = |id: &str| {
        PeerReply::Stream(vec![
            json!({"type":"acknowledged","subscriptionId":id}),
            json!({"type":"complete","subscriptionId":id,"completedAtMs":1,"cancelled":false}),
        ])
    };
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(stream("a")), Ok(stream("b"))],
    ));
    let client = Arc::new(make_client(peer, 1));
    let cancellation = Cancellation::new();
    client
        .discover(&context("p", &cancellation, &[], 0))
        .unwrap();
    let ids = std::thread::scope(|scope| {
        let first = {
            let client = Arc::clone(&client);
            scope.spawn(move || {
                let cancellation = Cancellation::new();
                client
                    .subscribe(
                        "events/subscribe",
                        json!({"listen":1}),
                        &context("p", &cancellation, &[], 1),
                    )
                    .unwrap()
                    .id
            })
        };
        let second = {
            let client = Arc::clone(&client);
            scope.spawn(move || {
                let cancellation = Cancellation::new();
                client
                    .subscribe(
                        "events/subscribe",
                        json!({"listen":2}),
                        &context("p", &cancellation, &[], 1),
                    )
                    .unwrap()
                    .id
            })
        };
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(
        ids.into_iter().collect::<std::collections::BTreeSet<_>>(),
        ["a".into(), "b".into()].into()
    );
}

#[test]
fn policy_expiry_and_process_exit_invalidate_era() {
    let peer = Arc::new(Peer::new(
        "stdio",
        vec![
            Ok(discovery()),
            Err(BindingError::ProcessExited(9)),
            Ok(discovery()),
        ],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    client
        .discover(&context("p", &cancellation, &[], 0))
        .unwrap();
    assert!(matches!(
        client.subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 1)
        ),
        Err(ClientError::Binding(BindingError::ProcessExited(9)))
    ));
    client
        .discover(&context("p", &cancellation, &[], 2))
        .unwrap();
    assert_eq!(
        peer.methods(),
        ["server/discover", "events/subscribe", "server/discover"]
    );
}
