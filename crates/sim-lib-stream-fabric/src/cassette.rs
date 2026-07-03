//! Ledger-backed storage for content-addressed eval replies.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use sim_kernel::{ContentId, Error, EvalReply, HandleId, Ref, Result, effect_ledger::EffectLedger};

use crate::content_key::ContentKey;

/// Append and replay surface used by [`EvalCassette`].
///
/// The production adapter is [`EffectLedgerCassette`], which writes every
/// recorded key to the kernel [`EffectLedger`] cassette table. The trait keeps
/// `EvalCassette` independent of a concrete persistence backend while retaining
/// fail-closed write semantics.
pub trait EvalCassetteLedger: Send + Sync {
    /// Appends one content key and reply pair.
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()>;

    /// Replays the ledger into key and reply pairs.
    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>>;
}

/// A kernel [`EffectLedger`] adapter for [`EvalCassette`].
///
/// The kernel ledger stores replay hints as `ContentId -> Ref`. This adapter
/// records that hint in the ledger and keeps the reply objects alongside it so a
/// cassette created from the adapter can rebuild its hot-path map immediately.
#[derive(Default)]
pub struct EffectLedgerCassette {
    ledger: Mutex<EffectLedger>,
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EffectLedgerCassette {
    /// Creates an empty adapter backed by a fresh [`EffectLedger`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an adapter around an existing [`EffectLedger`].
    pub fn with_ledger(ledger: EffectLedger) -> Self {
        Self {
            ledger: Mutex::new(ledger),
            entries: Mutex::new(Vec::new()),
        }
    }

    fn cassette_result(&self, key: &ContentKey) -> Result<Option<Ref>> {
        let key_id = content_key_id(key)?;
        Ok(lock(&self.ledger, "effect ledger")?
            .cassette_result(&key_id)
            .cloned())
    }
}

impl EvalCassetteLedger for EffectLedgerCassette {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
        if self.cassette_result(key)?.is_some() {
            return Ok(());
        }

        let key_id = content_key_id(key)?;
        let result_ref = Ref::Handle(HandleId::fresh());
        lock(&self.ledger, "effect ledger")?.insert_cassette_result(key_id, result_ref);
        lock(&self.entries, "cassette entries")?.push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>> {
        Ok(lock(&self.entries, "cassette entries")?.clone())
    }
}

/// An [`EffectLedger`]-backed cache of content-addressed eval replies.
///
/// The hot path is an in-memory [`BTreeMap`]. Recording a new reply writes to
/// the configured ledger first, then updates the in-memory map. Constructing a
/// cassette with [`EvalCassette::from_ledger`] replays ledger entries into a
/// fresh map.
pub struct EvalCassette {
    ledger: Arc<dyn EvalCassetteLedger>,
    cache: Mutex<BTreeMap<ContentKey, EvalReply>>,
}

impl EvalCassette {
    /// Creates an empty cassette backed by `ledger`.
    pub fn new(ledger: Arc<dyn EvalCassetteLedger>) -> Self {
        Self {
            ledger,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Replays `ledger` into a fresh in-memory map.
    pub fn from_ledger(ledger: Arc<dyn EvalCassetteLedger>) -> Result<Self> {
        let mut cache = BTreeMap::new();
        for (key, reply) in ledger.replay_eval_results()? {
            cache.insert(key, reply);
        }
        Ok(Self {
            ledger,
            cache: Mutex::new(cache),
        })
    }

    /// Returns a cached reply for `key`.
    pub fn get(&self, key: &ContentKey) -> Option<EvalReply> {
        self.cache.lock().ok()?.get(key).cloned()
    }

    /// Records `reply` under `key`, writing the ledger before updating cache.
    pub fn record(&self, key: ContentKey, reply: EvalReply) -> Result<()> {
        let mut cache = lock(&self.cache, "eval cassette cache")?;
        if cache.contains_key(&key) {
            return Ok(());
        }
        self.ledger.append_eval_result(&key, &reply)?;
        cache.insert(key, reply);
        Ok(())
    }

    /// Returns the number of cached replies.
    pub fn len(&self) -> usize {
        self.cache.lock().map_or(0, |cache| cache.len())
    }

    /// Returns whether the cassette has no cached replies.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn content_key_id(key: &ContentKey) -> Result<ContentId> {
    key.datum().content_id()
}

fn lock<'a, T>(mutex: &'a Mutex<T>, name: &str) -> Result<MutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|_| Error::Eval(format!("{name} mutex poisoned")))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sim_kernel::{
        CapabilityName, Consistency, Cx, EvalMode, EvalRequest, Expr, Value, testing::bare_cx as cx,
    };

    use super::*;

    #[derive(Default)]
    struct MemoryLedger {
        entries: Mutex<Vec<(ContentKey, EvalReply)>>,
        writes: Mutex<usize>,
    }

    impl MemoryLedger {
        fn push_existing(&self, key: ContentKey, reply: EvalReply) {
            self.entries.lock().unwrap().push((key, reply));
        }

        fn writes(&self) -> usize {
            *self.writes.lock().unwrap()
        }
    }

    impl EvalCassetteLedger for MemoryLedger {
        fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
            *self.writes.lock().unwrap() += 1;
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

    fn key(expr: &str) -> ContentKey {
        ContentKey::from_request(&request(expr))
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
    fn record_and_get_returns_stored_reply() {
        let mut cx = cx();
        let ledger = Arc::new(MemoryLedger::default());
        let cassette = EvalCassette::new(ledger);
        let key = key("record");
        let reply = reply(&mut cx, "stored");

        cassette.record(key.clone(), reply.clone()).unwrap();

        let stored = cassette.get(&key).unwrap();
        assert_eq!(
            value_display(&mut cx, &stored.value),
            value_display(&mut cx, &reply.value)
        );
    }

    #[test]
    fn duplicate_record_is_idempotent() {
        let mut cx = cx();
        let ledger = Arc::new(MemoryLedger::default());
        let cassette = EvalCassette::new(ledger.clone());
        let key = key("duplicate");
        let first = reply(&mut cx, "first");

        cassette.record(key.clone(), first.clone()).unwrap();
        cassette
            .record(key.clone(), reply(&mut cx, "second"))
            .unwrap();

        assert_eq!(ledger.writes(), 1);
        let stored = cassette.get(&key).unwrap();
        assert_eq!(
            value_display(&mut cx, &stored.value),
            value_display(&mut cx, &first.value)
        );
    }

    #[test]
    fn from_ledger_replays_prior_entries() {
        let mut cx = cx();
        let ledger = Arc::new(MemoryLedger::default());
        let first_key = key("first");
        let second_key = key("second");
        let first = reply(&mut cx, "one");
        let second = reply(&mut cx, "two");
        ledger.push_existing(first_key.clone(), first.clone());
        ledger.push_existing(second_key.clone(), second.clone());

        let cassette = EvalCassette::from_ledger(ledger).unwrap();

        assert_eq!(cassette.len(), 2);
        assert_eq!(
            value_display(&mut cx, &cassette.get(&first_key).unwrap().value),
            value_display(&mut cx, &first.value)
        );
        assert_eq!(
            value_display(&mut cx, &cassette.get(&second_key).unwrap().value),
            value_display(&mut cx, &second.value)
        );
    }
}
