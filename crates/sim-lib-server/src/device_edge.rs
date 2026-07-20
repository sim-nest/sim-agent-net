//! Route-transparent device edge sessions.

use sim_kernel::{Error, EventKind, EventLedger, Expr, Ref, Result, Symbol};

/// Stable reference to the ledger owned by a device edge session.
pub type LedgerRef = Ref;

/// Transport route currently carrying a device edge session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// Direct local link.
    Direct,
    /// Relayed link through another host.
    Relay,
    /// Host-local in-process link.
    Local,
    /// Any other route token carried as open data.
    Custom(Symbol),
}

impl LinkKind {
    /// Returns the stable route symbol.
    pub fn as_symbol(&self) -> Symbol {
        match self {
            Self::Direct => link_symbol("direct"),
            Self::Relay => link_symbol("relay"),
            Self::Local => link_symbol("local"),
            Self::Custom(symbol) => symbol.clone(),
        }
    }

    /// Builds a route kind from a symbol.
    pub fn from_symbol(symbol: Symbol) -> Self {
        if symbol.namespace.as_deref() == Some("device/link") {
            match symbol.name.as_ref() {
                "direct" => Self::Direct,
                "relay" => Self::Relay,
                "local" => Self::Local,
                _ => Self::Custom(symbol),
            }
        } else {
            Self::Custom(symbol)
        }
    }

    /// Encodes this route as expression data.
    pub fn to_expr(&self) -> Expr {
        Expr::Symbol(self.as_symbol())
    }
}

/// A device edge session whose identity survives transport route changes.
///
/// The session owns a stable id, a stable ledger reference, a mutable current
/// link, an event ledger, and optional visible consent data. Rebinding changes
/// only the route; the id, ledger reference, event history, and bound consent
/// remain attached to the same session.
#[derive(Clone, Debug)]
pub struct DeviceEdgeSession {
    id: Symbol,
    link: LinkKind,
    ledger: LedgerRef,
    events: EventLedger,
    consent: Option<Expr>,
}

impl DeviceEdgeSession {
    /// Creates a device edge session using the id as the ledger reference.
    pub fn new(id: Symbol, link: LinkKind) -> Result<Self> {
        let ledger = Ref::Symbol(id.clone());
        Self::with_ledger(id, link, ledger)
    }

    /// Creates a device edge session with an explicit ledger reference.
    pub fn with_ledger(id: Symbol, link: LinkKind, ledger: LedgerRef) -> Result<Self> {
        let mut session = Self {
            id,
            link,
            ledger,
            events: EventLedger::new(),
            consent: None,
        };
        session.record_event("open")?;
        Ok(session)
    }

    /// Returns the stable session id.
    pub fn id(&self) -> &Symbol {
        &self.id
    }

    /// Returns the current route carrying this session.
    pub fn link(&self) -> &LinkKind {
        &self.link
    }

    /// Returns the stable ledger reference.
    pub fn ledger_ref(&self) -> &LedgerRef {
        &self.ledger
    }

    /// Returns the session event ledger.
    pub fn event_ledger(&self) -> &EventLedger {
        &self.events
    }

    /// Returns the visible consent data bound to the session, if present.
    pub fn bound_consent(&self) -> Option<&Expr> {
        self.consent.as_ref()
    }

    /// Returns the session symbol embedded in the bound consent data, if any.
    pub fn bound_consent_session(&self) -> Option<Symbol> {
        self.consent
            .as_ref()
            .and_then(|consent| sim_value::access::field_sym(consent, "session"))
    }

    /// Binds visible consent data to the session and records the ledger event.
    pub fn bind_consent(&mut self, consent: Expr) -> Result<()> {
        match sim_value::access::field_sym(&consent, "session") {
            Some(session) if session == self.id => {}
            Some(session) => {
                return Err(Error::HostError(format!(
                    "consent session '{session}' does not match device edge session '{}'",
                    self.id
                )));
            }
            None => {
                return Err(Error::HostError(
                    "device edge consent is missing session".to_owned(),
                ));
            }
        }
        self.consent = Some(consent);
        self.record_event("consent-bound")
    }

    /// Swaps the transport route without changing the session identity.
    pub fn rebind(&mut self, link: LinkKind) -> Result<()> {
        self.link = link;
        self.record_event("rebind")
    }

    fn record_event(&mut self, name: &str) -> Result<()> {
        self.events.push(
            self.ledger.clone(),
            EventKind::Trace(Ref::Symbol(Symbol::qualified("device/edge", name))),
        )?;
        Ok(())
    }
}

fn link_symbol(name: &str) -> Symbol {
    Symbol::qualified("device/link", name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use sim_value::build;

    #[test]
    fn device_session_survives_route_swap() {
        let id = Symbol::qualified("device/session", "route-swap");
        let consent = build::map(vec![
            ("kind", build::qsym("device", "consent-receipt")),
            ("session", Expr::Symbol(id.clone())),
            ("seq", build::uint(7)),
        ]);
        let mut session = DeviceEdgeSession::new(id.clone(), LinkKind::Direct).unwrap();
        session.bind_consent(consent.clone()).unwrap();

        let ledger = session.ledger_ref().clone();
        let events_before = session.event_ledger().len_for_run(&ledger);
        session.rebind(LinkKind::Relay).unwrap();

        assert_eq!(session.id(), &id);
        assert_eq!(session.ledger_ref(), &ledger);
        assert_eq!(session.link(), &LinkKind::Relay);
        assert_eq!(session.bound_consent(), Some(&consent));
        assert_eq!(session.bound_consent_session(), Some(id));
        assert_eq!(
            session.event_ledger().len_for_run(&ledger),
            events_before + 1
        );
    }

    #[test]
    fn consent_must_match_device_session() {
        let id = Symbol::qualified("device/session", "route-swap");
        let other = Symbol::qualified("device/session", "other");
        let consent = build::map(vec![
            ("kind", build::qsym("device", "consent-receipt")),
            ("session", Expr::Symbol(other)),
            ("seq", build::uint(7)),
        ]);
        let mut session = DeviceEdgeSession::new(id, LinkKind::Direct).unwrap();

        let err = session.bind_consent(consent).unwrap_err();

        assert!(matches!(
            err,
            Error::HostError(message) if message.contains("does not match")
        ));
        assert!(session.bound_consent().is_none());
    }
}
