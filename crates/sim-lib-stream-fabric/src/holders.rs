//! Grow-only content holder index.
//!
//! A holder index answers "who holds content id X" for
//! [`ContentAddressedFabric`](crate::ContentAddressedFabric). It carries only
//! holder facts, never content values or transport policy.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use sim_kernel::{Error, Result, Symbol};

use crate::{ContentKey, HeldContent};

/// A grow-only index of which nodes hold which content ids.
///
/// Content never mutates, so a holder fact is monotonic: it is either useful or
/// stale because the holder is unreachable. Merge is set union, giving the
/// index convergence without coordination or consensus. A stale announcement
/// degrades to a peer miss and the fabric continues to any other holder.
#[derive(Default)]
pub struct HoldingIndex {
    holders: Mutex<BTreeMap<ContentKey, BTreeSet<Symbol>>>,
}

impl HoldingIndex {
    /// Announces that `fact.holder` holds `fact.key`.
    ///
    /// Re-announcing the same fact is idempotent.
    pub fn announce(&self, fact: HeldContent) -> Result<()> {
        lock(&self.holders)?
            .entry(fact.key)
            .or_default()
            .insert(fact.holder);
        Ok(())
    }

    /// Returns holders announced for `key`, sorted by symbol order.
    pub fn holders_of(&self, key: &ContentKey) -> Vec<Symbol> {
        self.holders
            .lock()
            .ok()
            .and_then(|holders| holders.get(key).cloned())
            .map(|holders| holders.into_iter().collect())
            .unwrap_or_default()
    }

    /// Merges all facts from `other` by grow-only set union.
    pub fn merge(&self, other: &HoldingIndex) -> Result<()> {
        let other = lock(&other.holders)?;
        let mut holders = lock(&self.holders)?;
        for (key, nodes) in other.iter() {
            holders
                .entry(key.clone())
                .or_default()
                .extend(nodes.iter().cloned());
        }
        Ok(())
    }
}

fn lock(
    holders: &Mutex<BTreeMap<ContentKey, BTreeSet<Symbol>>>,
) -> Result<MutexGuard<'_, BTreeMap<ContentKey, BTreeSet<Symbol>>>> {
    holders
        .lock()
        .map_err(|_| Error::Eval("holding index mutex poisoned".to_owned()))
}

#[cfg(test)]
mod tests {
    use sim_kernel::{CapabilityName, Consistency, EvalMode, EvalRequest, Expr};

    use super::*;

    fn key(expr: &str) -> ContentKey {
        ContentKey::from_request(&EvalRequest {
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
        })
    }

    #[test]
    fn announce_is_idempotent_and_sorted() {
        let index = HoldingIndex::default();
        let key = key("shared");

        index
            .announce(HeldContent::new(Symbol::new("node-b"), key.clone()))
            .unwrap();
        index
            .announce(HeldContent::new(Symbol::new("node-a"), key.clone()))
            .unwrap();
        index
            .announce(HeldContent::new(Symbol::new("node-b"), key.clone()))
            .unwrap();

        assert_eq!(
            index.holders_of(&key),
            vec![Symbol::new("node-a"), Symbol::new("node-b")]
        );
    }

    #[test]
    fn merge_converges_by_set_union() {
        let key = key("shared");
        let left = HoldingIndex::default();
        let right = HoldingIndex::default();

        left.announce(HeldContent::new(Symbol::new("node-a"), key.clone()))
            .unwrap();
        right
            .announce(HeldContent::new(Symbol::new("node-b"), key.clone()))
            .unwrap();

        left.merge(&right).unwrap();
        right.merge(&left).unwrap();

        let expected = vec![Symbol::new("node-a"), Symbol::new("node-b")];
        assert_eq!(left.holders_of(&key), expected);
        assert_eq!(right.holders_of(&key), expected);
    }
}
