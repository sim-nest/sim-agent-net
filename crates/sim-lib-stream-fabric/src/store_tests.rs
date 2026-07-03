use std::sync::{Arc, Mutex};

use sim_kernel::{
    CapabilityName, Consistency, Cx, Error, EvalFabric, EvalFabricRef, EvalMode, EvalReply,
    EvalRequest, Expr, Result, Symbol, Value, testing::bare_cx as cx,
};

use crate::{
    ContentAddressedFabric, ContentKey, ContentPeer, ContentServeFabric, EvalCassette,
    EvalCassetteLedger, HeldContent, HoldingIndex,
};

#[derive(Default)]
struct MemoryLedger {
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EvalCassetteLedger for MemoryLedger {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>> {
        Ok(self.entries.lock().unwrap().clone())
    }
}

fn cassette() -> Arc<EvalCassette> {
    Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())))
}

fn serve(cassette: Arc<EvalCassette>) -> EvalFabricRef {
    Arc::new(ContentServeFabric::new(cassette))
}

fn request(expr: &str) -> EvalRequest {
    request_with_cap(expr, "fabric.test")
}

fn request_with_cap(expr: &str, cap: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(expr.to_owned()),
        result_shape: None,
        required_capabilities: vec![CapabilityName::new(cap)],
        deadline: None,
        consistency: Consistency::LocalFirst,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn reply(cx: &mut Cx, value: &str) -> EvalReply {
    EvalReply {
        value: cx.factory().string(value.to_owned()).unwrap(),
        diagnostics: Vec::new(),
        trace: None,
    }
}

fn value_display(cx: &mut Cx, value: &Value) -> String {
    value.object().display(cx).unwrap()
}

struct CountingHome {
    reply: EvalReply,
    calls: Arc<Mutex<usize>>,
}

impl CountingHome {
    fn new(reply: EvalReply) -> Self {
        Self {
            reply,
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn fabric(&self) -> EvalFabricRef {
        Arc::new(Self {
            reply: self.reply.clone(),
            calls: self.calls.clone(),
        })
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl EvalFabric for CountingHome {
    fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.reply.clone())
    }
}

struct DenyAllHome;

impl EvalFabric for DenyAllHome {
    fn realize(&self, _cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        Err(Error::CapabilityDenied {
            capability: request.required_capabilities[0].clone(),
        })
    }
}

struct FailingHome;

impl EvalFabric for FailingHome {
    fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        Err(Error::Eval("home compute failed".to_owned()))
    }
}

struct FailedServe {
    calls: Arc<Mutex<usize>>,
}

impl FailedServe {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn fabric(&self) -> EvalFabricRef {
        Arc::new(Self {
            calls: self.calls.clone(),
        })
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl EvalFabric for FailedServe {
    fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        *self.calls.lock().unwrap() += 1;
        Err(Error::Eval("peer unreachable".to_owned()))
    }
}

struct CountingServe {
    cassette: Arc<EvalCassette>,
    calls: Arc<Mutex<usize>>,
}

impl CountingServe {
    fn new(cassette: Arc<EvalCassette>) -> Self {
        Self {
            cassette,
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn fabric(&self) -> EvalFabricRef {
        Arc::new(Self {
            cassette: self.cassette.clone(),
            calls: self.calls.clone(),
        })
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl EvalFabric for CountingServe {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        *self.calls.lock().unwrap() += 1;
        ContentServeFabric::new(self.cassette.clone()).realize(cx, request)
    }
}

#[test]
fn request_resolves_from_a_peer_holder_with_no_central_router() {
    let mut cx = cx();
    let request = request("shared-work");
    let key = ContentKey::from_request(&request);
    let cas_a = cassette();
    let cas_b = cassette();
    let cas_c = cassette();
    let held_reply = reply(&mut cx, "from-b");
    cas_b.record(key.clone(), held_reply.clone()).unwrap();

    let fabric_a = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas_a.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-b"), serve(cas_b.clone())),
            ContentPeer::new(Symbol::new("node-c"), serve(cas_c.clone())),
        ],
    );

    let resolved = fabric_a.realize(&mut cx, request).unwrap();

    assert_eq!(
        value_display(&mut cx, &resolved.value),
        value_display(&mut cx, &held_reply.value)
    );
    assert!(cas_a.get(&key).is_some(), "node A becomes a holder");
    assert!(
        cas_c.get(&key).is_none(),
        "node C was not selected through a route table"
    );
}

#[test]
fn unheld_content_is_computed_on_home_then_becomes_locally_held() {
    let mut cx = cx();
    let home = CountingHome::new(reply(&mut cx, "computed"));
    let cas = cassette();
    let fabric = ContentAddressedFabric::new(Symbol::new("node-a"), cas.clone(), vec![])
        .with_home(home.fabric());
    let request = request("fresh-work");
    let key = ContentKey::from_request(&request);

    let first = fabric.realize(&mut cx, request.clone()).unwrap();
    let second = fabric.realize(&mut cx, request).unwrap();

    assert_eq!(
        value_display(&mut cx, &first.value),
        value_display(&mut cx, &second.value)
    );
    assert_eq!(home.calls(), 1, "second call is a local hold");
    assert!(cas.get(&key).is_some(), "computed reply becomes held");
}

#[test]
fn home_capability_denial_is_not_recorded_as_held() {
    let mut cx = cx();
    let cas = cassette();
    let fabric = ContentAddressedFabric::new(Symbol::new("node-a"), cas.clone(), vec![])
        .with_home(Arc::new(DenyAllHome));
    let request = request_with_cap("blocked", "fabric.denied");

    let Err(err) = fabric.realize(&mut cx, request.clone()) else {
        panic!("capability denial must propagate");
    };

    assert!(matches!(err, Error::CapabilityDenied { .. }));
    assert!(cas.get(&ContentKey::from_request(&request)).is_none());
}

#[test]
fn home_compute_error_is_not_recorded_as_held() {
    let mut cx = cx();
    let cas = cassette();
    let fabric = ContentAddressedFabric::new(Symbol::new("node-a"), cas.clone(), vec![])
        .with_home(Arc::new(FailingHome));
    let request = request("failing-work");

    let Err(err) = fabric.realize(&mut cx, request.clone()) else {
        panic!("compute error must propagate");
    };

    assert!(format!("{err}").contains("home compute failed"));
    assert!(cas.get(&ContentKey::from_request(&request)).is_none());
}

#[test]
fn third_node_resolves_content_after_first_node_computes_and_holds_it() {
    let mut cx = cx();
    let home = CountingHome::new(reply(&mut cx, "computed-on-a"));
    let cas_a = cassette();
    let cas_c = cassette();
    let request = request("fleet-work");

    let fabric_a = ContentAddressedFabric::new(Symbol::new("node-a"), cas_a.clone(), vec![])
        .with_home(home.fabric());
    let first = fabric_a.realize(&mut cx, request.clone()).unwrap();

    let fabric_c = ContentAddressedFabric::new(
        Symbol::new("node-c"),
        cas_c.clone(),
        vec![ContentPeer::new(
            Symbol::new("node-a"),
            serve(cas_a.clone()),
        )],
    );
    let from_peer = fabric_c.realize(&mut cx, request.clone()).unwrap();

    assert_eq!(home.calls(), 1);
    assert_eq!(
        value_display(&mut cx, &first.value),
        value_display(&mut cx, &from_peer.value)
    );
    assert!(cas_c.get(&ContentKey::from_request(&request)).is_some());
}

#[test]
fn partitioned_holder_is_skipped_and_another_holder_answers() {
    let mut cx = cx();
    let request = request("shared-partitioned-work");
    let key = ContentKey::from_request(&request);
    let cas_a = cassette();
    let cas_b = cassette();
    let cas_c = cassette();
    let failed_b = FailedServe::new();
    cas_b.record(key.clone(), reply(&mut cx, "from-b")).unwrap();
    cas_c.record(key.clone(), reply(&mut cx, "from-c")).unwrap();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas_a.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-b"), failed_b.fabric()),
            ContentPeer::new(Symbol::new("node-c"), serve(cas_c.clone())),
        ],
    );

    let resolved = fabric.realize(&mut cx, request).unwrap();

    assert_eq!(value_display(&mut cx, &resolved.value), "from-c");
    assert_eq!(failed_b.calls(), 1);
    assert!(cas_b.get(&key).is_some(), "node B's hold is unreachable");
    assert!(cas_a.get(&key).is_some(), "node A becomes a holder");
}

#[test]
fn two_holders_of_same_content_id_return_byte_identical_replies() {
    let mut cx = cx();
    let request = request("coherent-work");
    let key = ContentKey::from_request(&request);
    let cas_a = cassette();
    let cas_b = cassette();
    let cas_c = cassette();
    let immutable_reply = reply(&mut cx, "same-datum");
    cas_b.record(key.clone(), immutable_reply.clone()).unwrap();
    cas_c.record(key.clone(), immutable_reply.clone()).unwrap();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas_a,
        vec![ContentPeer::new(
            Symbol::new("node-b"),
            serve(cas_b.clone()),
        )],
    );
    let from_b = fabric.realize(&mut cx, request.clone()).unwrap();
    let from_c = serve(cas_c).realize(&mut cx, request).unwrap();

    assert_eq!(from_b.value, from_c.value);
    assert_eq!(from_b.diagnostics, from_c.diagnostics);
    assert_eq!(from_b.trace, from_c.trace);
    assert_eq!(value_display(&mut cx, &from_b.value), "same-datum");
}

#[test]
fn locally_held_content_survives_total_partition_without_network_contact() {
    let mut cx = cx();
    let request = request("cached-before-partition");
    let key = ContentKey::from_request(&request);
    let cas = cassette();
    let failed_b = FailedServe::new();
    let failed_c = FailedServe::new();
    cas.record(key, reply(&mut cx, "local-v")).unwrap();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas,
        vec![
            ContentPeer::new(Symbol::new("node-b"), failed_b.fabric()),
            ContentPeer::new(Symbol::new("node-c"), failed_c.fabric()),
        ],
    );

    let resolved = fabric.realize(&mut cx, request).unwrap();

    assert_eq!(value_display(&mut cx, &resolved.value), "local-v");
    assert_eq!(failed_b.calls(), 0, "local hold avoids peer contact");
    assert_eq!(failed_c.calls(), 0, "local hold avoids peer contact");
}

#[test]
fn total_partition_without_local_hold_or_home_fails_closed() {
    let mut cx = cx();
    let request = request("missing-in-total-partition");
    let key = ContentKey::from_request(&request);
    let cas = cassette();
    let failed_b = FailedServe::new();
    let failed_c = FailedServe::new();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-b"), failed_b.fabric()),
            ContentPeer::new(Symbol::new("node-c"), failed_c.fabric()),
        ],
    );

    let Err(err) = fabric.realize(&mut cx, request) else {
        panic!("total partition without a holder must fail closed");
    };

    assert!(format!("{err}").contains("no holder"));
    assert_eq!(failed_b.calls(), 1);
    assert_eq!(failed_c.calls(), 1);
    assert!(cas.get(&key).is_none(), "no phantom reply is recorded");
}

#[test]
fn gossip_merge_lets_a_node_ask_only_the_announced_holder() {
    let mut cx = cx();
    let request = request("gossip-shared-work");
    let key = ContentKey::from_request(&request);
    let cas_a = cassette();
    let cas_b = cassette();
    let cas_c = cassette();
    let serve_b = CountingServe::new(cas_b.clone());
    let serve_c = CountingServe::new(cas_c.clone());
    cas_b.record(key.clone(), reply(&mut cx, "from-b")).unwrap();

    let on_b = HoldingIndex::default();
    on_b.announce(HeldContent::new(Symbol::new("node-b"), key.clone()))
        .unwrap();
    let on_a = Arc::new(HoldingIndex::default());
    on_a.merge(&on_b).unwrap();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas_a.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-c"), serve_c.fabric()),
            ContentPeer::new(Symbol::new("node-b"), serve_b.fabric()),
        ],
    )
    .with_holding_index(on_a.clone());

    assert_eq!(on_a.holders_of(&key), vec![Symbol::new("node-b")]);

    let resolved = fabric.realize(&mut cx, request).unwrap();

    assert_eq!(value_display(&mut cx, &resolved.value), "from-b");
    assert_eq!(serve_b.calls(), 1, "announced holder is asked");
    assert_eq!(serve_c.calls(), 0, "unannounced peer is not fanned out");
    assert!(cas_a.get(&key).is_some(), "node A becomes a holder");
    assert_eq!(
        fabric.holding_index().holders_of(&key),
        vec![Symbol::new("node-a"), Symbol::new("node-b")]
    );
}

#[test]
fn gossip_stale_announcement_falls_back_to_miss_and_skip_path() {
    let mut cx = cx();
    let request = request("gossip-stale-work");
    let key = ContentKey::from_request(&request);
    let cas_a = cassette();
    let cas_c = cassette();
    let failed_b = FailedServe::new();
    let serve_c = CountingServe::new(cas_c.clone());
    cas_c.record(key.clone(), reply(&mut cx, "from-c")).unwrap();

    let on_a = Arc::new(HoldingIndex::default());
    on_a.announce(HeldContent::new(Symbol::new("node-b"), key.clone()))
        .unwrap();

    let fabric = ContentAddressedFabric::new(
        Symbol::new("node-a"),
        cas_a.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-b"), failed_b.fabric()),
            ContentPeer::new(Symbol::new("node-c"), serve_c.fabric()),
        ],
    )
    .with_holding_index(on_a);

    let resolved = fabric.realize(&mut cx, request).unwrap();

    assert_eq!(value_display(&mut cx, &resolved.value), "from-c");
    assert_eq!(failed_b.calls(), 1, "stale announced holder is skipped");
    assert_eq!(serve_c.calls(), 1, "fallback asks the full peer list");
    assert!(
        cas_a.get(&key).is_some(),
        "node A records the recovered hold"
    );
}
