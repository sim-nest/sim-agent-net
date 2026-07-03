//! Content-addressed store contract types.
//!
//! The store layer keeps routing out of caller code. A request maps to a
//! [`ContentKey`], and any holder of that key can answer through the standard
//! [`EvalFabric`] surface. Local code still calls `realize`; transport handles
//! are wrapped as peer fabrics below this API.

use std::sync::Arc;

use sim_kernel::{Cx, Error, EvalFabric, EvalFabricRef, EvalReply, EvalRequest, Result, Symbol};

use crate::{EvalCassette, HoldingIndex, content_key::ContentKey};

/// A claim that one node holds one content id in its cassette.
///
/// This is the complete topology fact for the content-addressed store: not a
/// route, not a partition, just an immutable statement that `holder` can answer
/// any request whose [`ContentKey`] equals `key`. Because content never mutates,
/// a `HeldContent` fact is monotonic and never invalidated.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeldContent {
    /// The node that holds the content.
    pub holder: Symbol,
    /// The content-addressed request key held by the node.
    pub key: ContentKey,
}

impl HeldContent {
    /// Creates a held-content claim for `holder` and `key`.
    pub fn new(holder: Symbol, key: ContentKey) -> Self {
        Self { holder, key }
    }
}

/// Read side of the content-addressed store for one node.
///
/// This fabric answers a request only when its cassette already holds the
/// request's [`ContentKey`]. It never computes, never contacts another node,
/// and never invents a route; it is the question "do you hold content id X"
/// exposed as an [`EvalFabric`].
pub struct ContentServeFabric {
    cassette: Arc<EvalCassette>,
}

impl ContentServeFabric {
    /// Builds a serve-only fabric over `cassette`.
    pub fn new(cassette: Arc<EvalCassette>) -> Self {
        Self { cassette }
    }

    /// Returns the cassette this serve fabric reads from.
    pub fn cassette(&self) -> &Arc<EvalCassette> {
        &self.cassette
    }
}

impl EvalFabric for ContentServeFabric {
    fn realize(&self, _cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let key = ContentKey::from_request(&request);
        self.cassette
            .get(&key)
            .ok_or_else(|| Error::Eval("content not held by this node".to_owned()))
    }
}

/// A peer that may hold content in its local cassette.
pub struct ContentPeer {
    /// Stable node id for diagnostics and later holder-index announcements.
    pub node: Symbol,
    /// Serve-only fabric for the peer's held content.
    pub serve: EvalFabricRef,
}

impl ContentPeer {
    /// Builds a peer handle from `node` and its serve fabric.
    pub fn new(node: Symbol, serve: EvalFabricRef) -> Self {
        Self { node, serve }
    }
}

/// Content-addressed lookup fabric over local and peer cassettes.
///
/// This presents the fleet as one [`EvalFabric`]. Lookup checks the local
/// cassette first, asks peers announced by the [`HoldingIndex`] before falling
/// back to the configured peer order, records the first immutable reply locally,
/// and computes on the configured home fabric only when no holder answers.
/// Callers never select a node or a transport; "routing" is content lookup.
pub struct ContentAddressedFabric {
    node: Symbol,
    local: Arc<EvalCassette>,
    peers: Vec<ContentPeer>,
    home: Option<EvalFabricRef>,
    holders: Arc<HoldingIndex>,
}

impl ContentAddressedFabric {
    /// Builds a content-addressed lookup fabric for `node`.
    pub fn new(node: Symbol, local: Arc<EvalCassette>, peers: Vec<ContentPeer>) -> Self {
        Self {
            node,
            local,
            peers,
            home: None,
            holders: Arc::new(HoldingIndex::default()),
        }
    }

    /// Uses `holders` as this node's grow-only holder index.
    pub fn with_holding_index(mut self, holders: Arc<HoldingIndex>) -> Self {
        self.holders = holders;
        self
    }

    /// Records a home compute fabric handle for the miss path.
    pub fn with_home(mut self, home: EvalFabricRef) -> Self {
        self.home = Some(home);
        self
    }

    /// Returns this node's stable id.
    pub fn node(&self) -> &Symbol {
        &self.node
    }

    /// Returns the local cassette used for held-content lookup and caching.
    pub fn local_cassette(&self) -> &Arc<EvalCassette> {
        &self.local
    }

    /// Returns the configured peer list.
    pub fn peers(&self) -> &[ContentPeer] {
        &self.peers
    }

    /// Returns this node's grow-only holder index.
    pub fn holding_index(&self) -> &Arc<HoldingIndex> {
        &self.holders
    }

    fn ordered_peers(&self, key: &ContentKey) -> Vec<&ContentPeer> {
        let announced = self.holders.holders_of(key);
        let mut ordered = Vec::with_capacity(self.peers.len());

        for holder in &announced {
            if holder == &self.node {
                continue;
            }
            if let Some(peer) = self.peers.iter().find(|peer| &peer.node == holder) {
                ordered.push(peer);
            }
        }

        for peer in &self.peers {
            if peer.node == self.node || announced.iter().any(|holder| holder == &peer.node) {
                continue;
            }
            ordered.push(peer);
        }

        ordered
    }

    fn record_local_hold(&self, key: ContentKey, reply: EvalReply) -> Result<()> {
        self.local.record(key.clone(), reply)?;
        self.holders
            .announce(HeldContent::new(self.node.clone(), key))
    }
}

impl EvalFabric for ContentAddressedFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let key = ContentKey::from_request(&request);
        if let Some(reply) = self.local.get(&key) {
            return Ok(reply);
        }

        for peer in self.ordered_peers(&key) {
            if let Ok(reply) = peer.serve.realize(cx, request.clone()) {
                self.record_local_hold(key, reply.clone())?;
                return Ok(reply);
            }
        }

        if let Some(home) = &self.home {
            let reply = home.realize(cx, request)?;
            self.record_local_hold(key, reply.clone())?;
            return Ok(reply);
        }
        Err(Error::Eval(
            "no holder for content id and no home compute site".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sim_kernel::{
        CapabilityName, Consistency, Cx, EvalMode, EvalReply, EvalRequest, Expr, Result, Value,
        testing::bare_cx as cx,
    };

    use crate::cassette::EvalCassetteLedger;

    use super::*;

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

    fn mem_ledger() -> Arc<dyn EvalCassetteLedger> {
        Arc::new(MemoryLedger::default())
    }

    fn req(expr: &str, caps: &[&str]) -> EvalRequest {
        EvalRequest {
            expr: Expr::String(expr.to_owned()),
            result_shape: None,
            required_capabilities: caps
                .iter()
                .map(|capability| CapabilityName::new(*capability))
                .collect(),
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
    fn a_datum_is_the_same_from_any_node() {
        let key_a = ContentKey::from_request(&req("shared", &["fabric.test"]));
        let key_b = ContentKey::from_request(&req("shared", &["fabric.test"]));
        assert_eq!(key_a, key_b, "same request must key the same everywhere");

        let mut cx = cx();
        let reply = reply(&mut cx, "v");
        let node_a = EvalCassette::new(mem_ledger());
        let node_b = EvalCassette::new(mem_ledger());
        node_a.record(key_a.clone(), reply.clone()).unwrap();
        node_b.record(key_b.clone(), reply.clone()).unwrap();

        let stored_a = node_a.get(&key_a).unwrap();
        let stored_b = node_b.get(&key_b).unwrap();
        assert_eq!(stored_a.value, stored_b.value);
        assert_eq!(
            value_display(&mut cx, &stored_a.value),
            value_display(&mut cx, &stored_b.value)
        );
    }
}
