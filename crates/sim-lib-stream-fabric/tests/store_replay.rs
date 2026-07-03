use std::sync::{Arc, Mutex};

use sim_kernel::{
    CapabilityName, Consistency, Cx, Error, EvalFabric, EvalFabricRef, EvalMode, EvalReply,
    EvalRequest, Expr, Result, Symbol, Value, testing::bare_cx as cx,
};
use sim_lib_stream_fabric::{
    ContentAddressedFabric, ContentKey, ContentPeer, EvalCassette, EvalCassetteLedger,
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
        Err(Error::Eval("node lost".to_owned()))
    }
}

#[test]
fn content_store_replays_holdings_after_multiple_node_losses() {
    let mut cx = cx();
    let ledger_a = Arc::new(MemoryLedger::default());
    let ledger_b = Arc::new(MemoryLedger::default());
    let ledger_c = Arc::new(MemoryLedger::default());
    let req_a = request("node-a-only");
    let req_b = request("node-b-survivor");
    let req_c = request("node-c-only");
    let req_replicated = request("node-a-and-b");
    let key_a = ContentKey::from_request(&req_a);
    let key_b = ContentKey::from_request(&req_b);
    let key_c = ContentKey::from_request(&req_c);
    let key_replicated = ContentKey::from_request(&req_replicated);

    {
        let cas_a = EvalCassette::new(ledger_a.clone());
        let cas_b = EvalCassette::new(ledger_b.clone());
        let cas_c = EvalCassette::new(ledger_c.clone());
        cas_a
            .record(key_a.clone(), reply(&mut cx, "from-a"))
            .unwrap();
        cas_b
            .record(key_b.clone(), reply(&mut cx, "from-b"))
            .unwrap();
        cas_c
            .record(key_c.clone(), reply(&mut cx, "from-c"))
            .unwrap();
        cas_a
            .record(key_replicated.clone(), reply(&mut cx, "shared-v"))
            .unwrap();
        cas_b
            .record(key_replicated.clone(), reply(&mut cx, "shared-v"))
            .unwrap();
    }

    let recovered_a = Arc::new(EvalCassette::from_ledger(ledger_a.clone()).unwrap());
    let recovered_b = Arc::new(EvalCassette::from_ledger(ledger_b.clone()).unwrap());
    let recovered_c = Arc::new(EvalCassette::from_ledger(ledger_c.clone()).unwrap());

    assert_local_replay(
        &mut cx,
        "node-a",
        recovered_a.clone(),
        req_a.clone(),
        "from-a",
    );
    assert_local_replay(
        &mut cx,
        "node-b",
        recovered_b.clone(),
        req_b.clone(),
        "from-b",
    );
    assert_local_replay(&mut cx, "node-c", recovered_c, req_c.clone(), "from-c");

    let lost_a = FailedServe::new();
    let lost_c = FailedServe::new();
    let survivor = ContentAddressedFabric::new(
        Symbol::new("node-b"),
        recovered_b.clone(),
        vec![
            ContentPeer::new(Symbol::new("node-a"), lost_a.fabric()),
            ContentPeer::new(Symbol::new("node-c"), lost_c.fabric()),
        ],
    );

    let survivor_only = survivor.realize(&mut cx, req_b).unwrap();
    assert_eq!(value_display(&mut cx, &survivor_only.value), "from-b");
    assert_eq!(lost_a.calls(), 0, "survivor-local content avoids peers");
    assert_eq!(lost_c.calls(), 0, "survivor-local content avoids peers");

    let replicated = survivor.realize(&mut cx, req_replicated).unwrap();
    assert_eq!(value_display(&mut cx, &replicated.value), "shared-v");

    let Err(lost_only) = survivor.realize(&mut cx, req_a.clone()) else {
        panic!("content only held by lost nodes must not be fabricated");
    };
    assert!(format!("{lost_only}").contains("no holder"));
    assert!(recovered_b.get(&key_a).is_none());

    let unknown = request("never-computed");
    let unknown_key = ContentKey::from_request(&unknown);
    let Err(unknown_err) = survivor.realize(&mut cx, unknown) else {
        panic!("unknown content must fail closed");
    };
    assert!(format!("{unknown_err}").contains("no holder"));
    assert!(recovered_b.get(&unknown_key).is_none());
}

fn assert_local_replay(
    cx: &mut Cx,
    node: &str,
    cassette: Arc<EvalCassette>,
    request: EvalRequest,
    expected: &str,
) {
    let failed_peer = FailedServe::new();
    let fabric = ContentAddressedFabric::new(
        Symbol::new(node),
        cassette,
        vec![ContentPeer::new(
            Symbol::new("lost-peer"),
            failed_peer.fabric(),
        )],
    );

    let resolved = fabric.realize(cx, request).unwrap();

    assert_eq!(value_display(cx, &resolved.value), expected);
    assert_eq!(
        failed_peer.calls(),
        0,
        "recovered local hold should not contact the network"
    );
}

fn request(expr: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(expr.to_owned()),
        result_shape: None,
        required_capabilities: vec![CapabilityName::new("fabric.test")],
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
