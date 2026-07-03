//! Cache-first relay fabric backed by an eval cassette.

use std::sync::Arc;

use sim_kernel::{Cx, EvalFabric, EvalReply, EvalRequest, Object, ObjectCompat, Result, Value};

use crate::{ContentKey, EvalCassette};

/// A cache-first [`EvalFabric`] that records successful replies in a cassette.
///
/// [`LedgeredRelayFabric`] derives a [`ContentKey`] from each request, returns a
/// cached reply without touching the inner fabric when one exists, and writes
/// every successful miss-through reply to the cassette. Capability policy stays
/// entirely inside the inner fabric: errors, including capability denials, are
/// propagated without recording a cassette result.
pub struct LedgeredRelayFabric<F> {
    inner: F,
    cassette: Arc<EvalCassette>,
}

impl<F: EvalFabric> LedgeredRelayFabric<F> {
    /// Builds a ledgered relay around `inner` and `cassette`.
    pub fn new(inner: F, cassette: Arc<EvalCassette>) -> Self {
        Self { inner, cassette }
    }

    /// Returns the cassette used for cache lookups and write-through records.
    pub fn cassette(&self) -> &Arc<EvalCassette> {
        &self.cassette
    }

    /// Returns the wrapped fabric.
    pub fn inner(&self) -> &F {
        &self.inner
    }
}

impl<F: EvalFabric + 'static> EvalFabric for LedgeredRelayFabric<F> {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let key = ContentKey::from_request(&request);
        if let Some(cached) = self.cassette.get(&key) {
            return Ok(cached);
        }

        let reply = self.inner.realize(cx, request)?;
        self.cassette.record(key, reply.clone())?;
        Ok(reply)
    }
}

impl<F: EvalFabric + 'static> Object for LedgeredRelayFabric<F> {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<ledgered-relay-fabric>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<F: EvalFabric + 'static> ObjectCompat for LedgeredRelayFabric<F> {
    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

/// Wraps `inner` and `cassette` as an opaque runtime value.
///
/// # Errors
///
/// Returns an error when the active factory cannot allocate the opaque value.
pub fn ledgered_relay_fabric_value<F: EvalFabric + 'static>(
    cx: &mut Cx,
    inner: F,
    cassette: Arc<EvalCassette>,
) -> Result<Value> {
    cx.factory()
        .opaque(Arc::new(LedgeredRelayFabric::new(inner, cassette)))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sim_kernel::{
        CapabilityName, Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr,
        Result, Value, testing::bare_cx as cx,
    };

    use crate::{EvalCassette, EvalCassetteLedger};

    use super::{LedgeredRelayFabric, ledgered_relay_fabric_value};

    struct CountingFabric {
        replies: Vec<EvalReply>,
        calls: Arc<Mutex<usize>>,
    }

    impl CountingFabric {
        fn new(replies: Vec<EvalReply>, calls: Arc<Mutex<usize>>) -> Self {
            Self { replies, calls }
        }
    }

    impl EvalFabric for CountingFabric {
        fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
            let mut calls = self.calls.lock().unwrap();
            let reply = self
                .replies
                .get(*calls)
                .or_else(|| self.replies.last())
                .expect("counting fabric must have at least one reply")
                .clone();
            *calls += 1;
            Ok(reply)
        }
    }

    struct DenyAll;

    impl EvalFabric for DenyAll {
        fn realize(&self, _cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
            Err(Error::CapabilityDenied {
                capability: request.required_capabilities[0].clone(),
            })
        }
    }

    #[derive(Default)]
    struct MemoryLedger {
        entries: Mutex<Vec<(crate::ContentKey, EvalReply)>>,
    }

    impl EvalCassetteLedger for MemoryLedger {
        fn append_eval_result(&self, key: &crate::ContentKey, reply: &EvalReply) -> Result<()> {
            self.entries
                .lock()
                .unwrap()
                .push((key.clone(), reply.clone()));
            Ok(())
        }

        fn replay_eval_results(&self) -> Result<Vec<(crate::ContentKey, EvalReply)>> {
            Ok(self.entries.lock().unwrap().clone())
        }
    }

    fn cassette() -> Arc<EvalCassette> {
        Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())))
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

    #[test]
    fn ledgered_relay_cache_miss_calls_inner_once_then_cache_hit_skips_inner() {
        let mut cx = cx();
        let calls = Arc::new(Mutex::new(0));
        let inner = CountingFabric::new(vec![reply(&mut cx, "first")], calls.clone());
        let fabric = LedgeredRelayFabric::new(inner, cassette());
        let request = request("same-expr");

        let first = fabric.realize(&mut cx, request.clone()).unwrap();
        let second = fabric.realize(&mut cx, request).unwrap();

        assert_eq!(*calls.lock().unwrap(), 1);
        assert_eq!(
            value_display(&mut cx, &second.value),
            value_display(&mut cx, &first.value)
        );
    }

    #[test]
    fn ledgered_relay_different_expressions_each_reach_inner_once() {
        let mut cx = cx();
        let calls = Arc::new(Mutex::new(0));
        let inner = CountingFabric::new(
            vec![reply(&mut cx, "first"), reply(&mut cx, "second")],
            calls.clone(),
        );
        let fabric = LedgeredRelayFabric::new(inner, cassette());

        fabric.realize(&mut cx, request("expr-a")).unwrap();
        fabric.realize(&mut cx, request("expr-b")).unwrap();

        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[test]
    fn ledgered_relay_capability_error_is_not_recorded_in_cassette() {
        let cassette = cassette();
        let fabric = LedgeredRelayFabric::new(DenyAll, cassette.clone());
        let mut cx = cx();

        let Err(err) = fabric.realize(&mut cx, request_with_cap("blocked", "fabric.denied")) else {
            panic!("capability error must propagate");
        };

        assert!(matches!(err, Error::CapabilityDenied { .. }));
        assert_eq!(cassette.len(), 0);
    }

    #[test]
    fn ledgered_relay_value_exposes_eval_fabric() {
        let mut cx = cx();
        let inner = CountingFabric::new(vec![reply(&mut cx, "value")], Arc::new(Mutex::new(0)));
        let value = ledgered_relay_fabric_value(&mut cx, inner, cassette()).unwrap();

        assert!(value.object().as_eval_fabric().is_some());
    }
}
