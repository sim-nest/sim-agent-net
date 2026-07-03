use std::sync::{Arc, Mutex};

use sim_kernel::{
    CapabilityName, Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr,
    Result, Value, testing::bare_cx as cx,
};
use sim_lib_stream_fabric::{ContentKey, EvalCassette, EvalCassetteLedger, LedgeredRelayFabric};

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

struct FailAfterN {
    remaining: Mutex<usize>,
    attempts: Arc<Mutex<usize>>,
    replies: Vec<EvalReply>,
}

impl FailAfterN {
    fn new(allowed_calls: usize, replies: Vec<EvalReply>) -> Self {
        Self {
            remaining: Mutex::new(allowed_calls),
            attempts: Arc::new(Mutex::new(0)),
            replies,
        }
    }

    fn attempts(&self) -> Arc<Mutex<usize>> {
        self.attempts.clone()
    }
}

impl EvalFabric for FailAfterN {
    fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        *self.attempts.lock().unwrap() += 1;
        let mut remaining = self.remaining.lock().unwrap();
        if *remaining == 0 {
            return Err(Error::Eval("node failed".to_owned()));
        }
        let index = self.replies.len().saturating_sub(*remaining);
        *remaining -= 1;
        Ok(self
            .replies
            .get(index)
            .or_else(|| self.replies.last())
            .expect("fail-after fixture must have at least one reply")
            .clone())
    }
}

#[test]
fn cassette_replay_recovers_prior_results_after_node_failure() {
    let mut cx = cx();
    let ledger = Arc::new(MemoryLedger::default());
    let first = reply(&mut cx, "result-a");
    let second = reply(&mut cx, "result-b");

    {
        let cassette = Arc::new(EvalCassette::new(ledger.clone()));
        let inner = FailAfterN::new(2, vec![first.clone(), second.clone()]);
        let fabric = LedgeredRelayFabric::new(inner, cassette);

        fabric.realize(&mut cx, request("req-a")).unwrap();
        fabric.realize(&mut cx, request("req-b")).unwrap();
    }

    let cassette = Arc::new(EvalCassette::from_ledger(ledger.clone()).unwrap());
    let dead_inner = FailAfterN::new(0, vec![reply(&mut cx, "unreachable")]);
    let attempts = dead_inner.attempts();
    let recovered = LedgeredRelayFabric::new(dead_inner, cassette);

    let cached_a = recovered.realize(&mut cx, request("req-a")).unwrap();
    let cached_b = recovered.realize(&mut cx, request("req-b")).unwrap();

    assert_eq!(
        value_display(&mut cx, &cached_a.value),
        value_display(&mut cx, &first.value)
    );
    assert_eq!(
        value_display(&mut cx, &cached_b.value),
        value_display(&mut cx, &second.value)
    );
    assert_eq!(*attempts.lock().unwrap(), 0);

    let Err(err) = recovered.realize(&mut cx, request("req-new")) else {
        panic!("unknown request against dead inner must fail");
    };

    assert!(matches!(err, Error::Eval(message) if message == "node failed"));
    assert_eq!(*attempts.lock().unwrap(), 1);
}

#[test]
fn cassette_never_returns_phantom_entry_for_unrecorded_key() {
    let ledger = Arc::new(MemoryLedger::default());
    let cassette = EvalCassette::from_ledger(ledger).unwrap();
    let key = ContentKey::from_request(&request("never-seen"));

    assert!(cassette.get(&key).is_none());
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
