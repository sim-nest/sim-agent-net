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
                .is_some_and(|new| {
                    new.amount <= old.amount
                        && old.sources.iter().all(|old_source| {
                            new.sources.iter().any(|new_source| {
                                new_source.owner == old_source.owner
                                    && new_source.amount <= old_source.amount
                            })
                        })
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(owner: &str, amount: u64) -> OwnedLimit {
        OwnedLimit {
            owner: Symbol::new(owner),
            unit: Symbol::new("tokens"),
            amount,
        }
    }

    #[test]
    fn narrower_ceiling_preserves_each_authority_sources_limit() {
        let prior = EffectiveCeiling::intersect([limit("caller", 100), limit("roadmap", 50)]);
        let narrowed = EffectiveCeiling::intersect([limit("caller", 40), limit("roadmap", 50)]);
        let masked_widening =
            EffectiveCeiling::intersect([limit("caller", 200), limit("roadmap", 50)]);
        let missing_source = EffectiveCeiling::intersect([limit("roadmap", 40)]);

        assert!(narrowed.is_narrower_than(&prior));
        assert!(!masked_widening.is_narrower_than(&prior));
        assert!(!missing_source.is_narrower_than(&prior));
    }
}
