use crate::{ProviderAdapter, ProviderFamilyCard, ProviderSeatCard, ProviderSeatId};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Data-driven registry of provider-family adapters and independently selectable seats.
///
/// Family and seat ordering is lexical and therefore presentation-only: callers must
/// select a seat explicitly before opening it. Discovery never chooses a preferred seat.
#[derive(Default)]
pub struct ProviderRegistry {
    adapters: BTreeMap<String, Arc<dyn ProviderAdapter>>,
    families: BTreeMap<String, ProviderFamilyCard>,
    seats: BTreeMap<String, ProviderSeatCard>,
}

impl ProviderRegistry {
    /// Constructs an empty provider registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one family adapter, refusing an already registered family.
    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) -> Result<()> {
        let card = adapter.family();
        let key = family_key(&card.family);
        if self.adapters.contains_key(&key) {
            return Err(Error::Eval(format!(
                "provider family {} is already registered",
                card.family
            )));
        }
        self.families.insert(key.clone(), card);
        self.adapters.insert(key, adapter);
        Ok(())
    }

    /// Removes a family adapter and all seats discovered through that family.
    pub fn remove_family(&mut self, family: &Symbol) -> Option<Arc<dyn ProviderAdapter>> {
        let key = family_key(family);
        self.families.remove(&key);
        self.seats.retain(|_, seat| seat.family != *family);
        self.adapters.remove(&key)
    }

    /// Lists every registered provider family without applying a preference order.
    pub fn families(&self) -> Vec<ProviderFamilyCard> {
        self.families.values().cloned().collect()
    }

    /// Lists every discovered provider seat without coalescing principals or endpoints.
    pub fn seats(&self) -> Vec<ProviderSeatCard> {
        self.seats.values().cloned().collect()
    }

    /// Looks up one registered family.
    pub fn show_family(&self, family: &Symbol) -> Option<ProviderFamilyCard> {
        self.families.get(&family_key(family)).cloned()
    }

    /// Looks up one discovered seat by its complete stable identity.
    pub fn show_seat(&self, seat: &ProviderSeatId) -> Option<ProviderSeatCard> {
        self.seats.get(&seat_key(seat)).cloned()
    }

    /// Runs every registered adapter's discovery operation and records every seat.
    ///
    /// A duplicate seat id is an error, including duplicates returned by different
    /// discovery passes. Use a fresh registry for a fresh snapshot.
    pub fn discover(&mut self, cx: &mut Cx, hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        let adapters = self.adapters.values().cloned().collect::<Vec<_>>();
        let mut discovered = Vec::new();
        for adapter in adapters {
            let family = adapter.family().family;
            for seat in adapter.discover(cx, hint.clone())? {
                if seat.family != family || seat.seat.family != family {
                    return Err(Error::Eval(format!(
                        "provider adapter {family} returned seat {} for a different family",
                        seat.seat
                    )));
                }
                self.insert_seat(seat.clone())?;
                discovered.push(seat);
            }
        }
        Ok(discovered)
    }

    /// Opens exactly the requested discovered seat through its family adapter.
    pub fn open(
        &self,
        cx: &mut Cx,
        seat: &ProviderSeatId,
        options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        let card = self
            .show_seat(seat)
            .ok_or_else(|| Error::Eval(format!("provider seat {seat} has not been discovered")))?;
        let adapter = self
            .adapters
            .get(&family_key(&card.family))
            .ok_or_else(|| {
                Error::Eval(format!("provider family {} is not registered", card.family))
            })?;
        adapter.open(cx, &card, options)
    }

    fn insert_seat(&mut self, seat: ProviderSeatCard) -> Result<()> {
        let key = seat_key(&seat.seat);
        if self.seats.contains_key(&key) {
            return Err(Error::Eval(format!(
                "provider seat {} is already registered",
                seat.seat
            )));
        }
        self.seats.insert(key, seat);
        Ok(())
    }

    /// Replaces a discovered seat explicitly in crate tests.
    #[cfg(test)]
    pub(crate) fn replace_seat_for_test(&mut self, seat: ProviderSeatCard) {
        self.seats.insert(seat_key(&seat.seat), seat);
    }
}

fn family_key(family: &Symbol) -> String {
    family.to_string()
}

fn seat_key(seat: &ProviderSeatId) -> String {
    seat.to_string()
}
