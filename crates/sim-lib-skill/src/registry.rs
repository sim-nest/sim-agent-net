use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use sim_citizen_derive::non_citizen;
use sim_kernel::{Cx, Error, Object, ObjectCompat, Result, Symbol, Value};

#[cfg(any(feature = "cache", feature = "cassette"))]
use crate::record::SkillAuditEntry;
use crate::{SkillCallable, SkillCard, SkillTransport, SkillTransportValue};

/// Live registry of skill transports and bound cards.
///
/// The registry owns the installed [`SkillTransport`]s and the [`SkillCard`]s
/// bound to them, and (with the `cache`/`cassette` features) the audit log of
/// skill calls. It is a cheaply clonable handle over shared state, so clones
/// observe the same registry. It is a live handle rather than a serializable
/// value; cards are projected through the `skill/Card` descriptor.
#[derive(Clone, Default)]
#[non_citizen(
    reason = "live skill registry handle; cards use skill/Card descriptor",
    kind = "handle",
    descriptor = "skill/Card"
)]
pub struct SkillRegistry {
    state: Arc<Mutex<SkillRegistryState>>,
}

#[derive(Default)]
struct SkillRegistryState {
    transports: BTreeMap<String, Arc<dyn SkillTransport>>,
    cards: BTreeMap<String, SkillCard>,
    #[cfg(any(feature = "cache", feature = "cassette"))]
    audit: Vec<SkillAuditEntry>,
}

impl SkillRegistry {
    /// Installs `transport`, keyed by its id, replacing any prior transport
    /// with the same id.
    pub fn install_transport(&self, transport: Arc<dyn SkillTransport>) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .transports
            .insert(transport.id().to_owned(), transport);
        Ok(())
    }

    /// Binds `card` to its installed transport and registers it as a callable.
    ///
    /// Resolves the card's transport, builds a [`SkillCallable`], registers it
    /// under the card's symbol, publishes its browse claims, and stores the
    /// card. Errors if the card's transport is not installed. Returns the
    /// registered callable value.
    pub fn bind_card(&self, cx: &mut Cx, card: SkillCard) -> Result<Value> {
        let transport = {
            let state = self
                .state
                .lock()
                .map_err(|_| Error::PoisonedLock("skill registry"))?;
            state
                .transports
                .get(&card.transport_id)
                .cloned()
                .ok_or_else(|| {
                    Error::Eval(format!("missing skill transport {}", card.transport_id))
                })?
        };
        #[cfg(any(feature = "cache", feature = "cassette"))]
        let skill_callable = SkillCallable::new_bound(card.clone(), transport, self.clone());
        #[cfg(not(any(feature = "cache", feature = "cassette")))]
        let skill_callable = SkillCallable::new(card.clone(), transport);
        let callable = cx.factory().opaque(Arc::new(skill_callable))?;
        cx.registry_mut()
            .register_function_value(card.symbol.clone(), callable.clone())?;
        crate::browse::publish_card_claims(cx, &card)?;
        self.state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .cards
            .insert(card.id.clone(), card);
        Ok(callable)
    }

    /// Returns all bound cards.
    pub fn cards(&self) -> Result<Vec<SkillCard>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .cards
            .values()
            .cloned()
            .collect())
    }

    /// Looks up a bound card by its `id`.
    pub fn card_by_id(&self, id: &str) -> Result<Option<SkillCard>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .cards
            .get(id)
            .cloned())
    }

    /// Looks up a bound card by its registered `symbol`.
    pub fn card_by_symbol(&self, symbol: &Symbol) -> Result<Option<SkillCard>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .cards
            .values()
            .find(|card| &card.symbol == symbol)
            .cloned())
    }

    #[cfg(any(feature = "cache", feature = "cassette"))]
    pub(crate) fn record_audit(&self, entry: SkillAuditEntry) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .audit
            .push(entry);
        Ok(())
    }

    #[cfg(any(feature = "cache", feature = "cassette"))]
    pub(crate) fn audit_values(&self, cx: &mut Cx) -> Result<Value> {
        let entries = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("skill registry"))?
            .audit
            .clone();
        let values = entries
            .iter()
            .map(|entry| entry.value(cx))
            .collect::<Result<Vec<_>>>()?;
        cx.factory().list(values)
    }
}

impl Object for SkillRegistry {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<skill-registry>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for SkillRegistry {
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/registry"))?,
            ),
            (
                Symbol::new("cards"),
                cx.factory().string(self.cards()?.len().to_string())?,
            ),
        ])
    }
}

/// Returns the symbol the [`SkillRegistry`] value is bound under.
pub fn skill_registry_symbol() -> Symbol {
    Symbol::qualified("skill", "registry")
}

/// Resolves the [`SkillRegistry`] installed in `cx`.
///
/// Errors if the registry value is missing or is not a registry.
pub fn skill_registry(cx: &mut Cx) -> Result<SkillRegistry> {
    let value = cx.resolve_value(&skill_registry_symbol())?;
    value
        .object()
        .downcast_ref::<SkillRegistry>()
        .cloned()
        .ok_or(Error::TypeMismatch {
            expected: "skill registry",
            found: "non-registry",
        })
}

pub(crate) fn transport_from_value(value: &Value) -> Result<Arc<dyn SkillTransport>> {
    value
        .object()
        .downcast_ref::<SkillTransportValue>()
        .map(SkillTransportValue::transport)
        .ok_or(Error::TypeMismatch {
            expected: "skill transport",
            found: "non-transport",
        })
}

pub(crate) fn card_from_value(cx: &mut Cx, value: &Value) -> Result<SkillCard> {
    if let Some(card) = value.object().downcast_ref::<SkillCard>() {
        return Ok(card.clone());
    }
    let expr = value.object().as_expr(cx)?;
    SkillCard::from_expr(&expr)
}
