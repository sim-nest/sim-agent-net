use std::collections::BTreeMap;

use sim_kernel::Symbol;

/// One authority-owned limit. Ownership is retained for audit even when a
/// different layer supplies the effective minimum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedLimit {
    pub owner: Symbol,
    pub unit: Symbol,
    pub amount: u64,
}

/// Pointwise ceiling plus every authority layer that contributed to it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectiveCeiling {
    pub limits: Vec<EffectiveLimit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveLimit {
    pub unit: Symbol,
    pub amount: u64,
    pub sources: Vec<OwnedLimit>,
}

impl EffectiveCeiling {
    /// Intersects caller, roadmap, conduct, site, sandbox, and operator layers
    /// without erasing either dimensions or provenance.
    pub fn intersect(layers: impl IntoIterator<Item = OwnedLimit>) -> Self {
        let mut grouped: BTreeMap<Symbol, Vec<OwnedLimit>> = BTreeMap::new();
        for limit in layers {
            grouped.entry(limit.unit.clone()).or_default().push(limit);
        }
        Self {
            limits: grouped
                .into_iter()
                .map(|(unit, mut sources)| {
                    sources.sort_by_key(|limit| limit.owner.to_string());
                    EffectiveLimit {
                        unit,
                        amount: sources.iter().map(|limit| limit.amount).min().unwrap_or(0),
                        sources,
                    }
                })
                .collect(),
        }
    }

    pub fn is_narrower_than(&self, prior: &Self) -> bool {
        prior.limits.iter().all(|old| {
            self.limits
                .iter()
                .find(|new| new.unit == old.unit)
                .is_some_and(|new| new.amount <= old.amount)
        })
    }
}
