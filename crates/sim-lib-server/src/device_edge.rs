//! Route-transparent device edge sessions.

use std::collections::VecDeque;

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
    /// Builds the preferred CI-safe import route.
    pub fn import() -> Self {
        Self::Custom(link_symbol("import"))
    }

    /// Builds the preferred CI-safe modeled route.
    pub fn modeled() -> Self {
        Self::Custom(link_symbol("modeled"))
    }

    /// Builds the standard BLE host route.
    pub fn ble_host() -> Self {
        Self::Custom(link_symbol("ble-host"))
    }

    /// Builds the phone relay route.
    pub const fn relay() -> Self {
        Self::Relay
    }

    /// Builds the Zepp Mini Program bridge route.
    pub fn zepp_bridge() -> Self {
        Self::Custom(link_symbol("zepp-bridge"))
    }

    /// Builds the Wi-Fi route that may only be used after bring-up verification.
    pub fn wifi_if_verified() -> Self {
        Self::Custom(link_symbol("wifi-if-verified"))
    }

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

/// Device profile and provider metadata bound to an edge session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceEdgeProfile {
    device: Symbol,
    provider: Symbol,
}

impl DeviceEdgeProfile {
    /// Builds edge-session profile metadata.
    pub fn new(device: Symbol, provider: Symbol) -> Self {
        Self { device, provider }
    }

    /// Returns the stable device symbol.
    pub fn device(&self) -> &Symbol {
        &self.device
    }

    /// Returns the provider symbol that supplies the session.
    pub fn provider(&self) -> &Symbol {
        &self.provider
    }

    /// Encodes the profile metadata as expression data.
    pub fn to_expr(&self) -> Expr {
        sim_value::build::map(vec![
            ("device", Expr::Symbol(self.device.clone())),
            ("provider", Expr::Symbol(self.provider.clone())),
        ])
    }
}

/// A device edge session whose identity survives transport route changes.
///
/// The session owns a stable id, a stable ledger reference, a mutable current
/// link, an optional device profile binding, an event ledger, a route order, a
/// command queue, and optional visible consent data. Rebinding changes only the
/// route; the id, ledger reference, event history, queued commands, profile, and
/// bound consent remain attached to the same session.
#[derive(Clone, Debug)]
pub struct DeviceEdgeSession {
    id: Symbol,
    link: LinkKind,
    ledger: LedgerRef,
    profile: Option<DeviceEdgeProfile>,
    route_order: Vec<LinkKind>,
    queued_commands: VecDeque<Expr>,
    flushed_commands: Vec<Expr>,
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
            profile: None,
            route_order: Vec::new(),
            queued_commands: VecDeque::new(),
            flushed_commands: Vec::new(),
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

    /// Returns the profile bound to this session, if one was registered.
    pub fn device_profile(&self) -> Option<&DeviceEdgeProfile> {
        self.profile.as_ref()
    }

    /// Binds provider/profile metadata to the session.
    pub fn bind_profile(&mut self, profile: DeviceEdgeProfile) -> Result<()> {
        self.profile = Some(profile);
        self.record_event("profile-bound")
    }

    /// Returns the preferred route order for this session.
    pub fn route_order(&self) -> &[LinkKind] {
        &self.route_order
    }

    /// Replaces the preferred route order.
    pub fn set_route_order(&mut self, route_order: Vec<LinkKind>) -> Result<()> {
        if route_order.is_empty() {
            return Err(Error::HostError(
                "device edge route order cannot be empty".to_owned(),
            ));
        }
        self.route_order = route_order;
        self.record_event("route-order")
    }

    /// Queues an actuator command while the current route is unavailable.
    pub fn queue_command(&mut self, command: Expr) -> Result<()> {
        self.queued_commands.push_back(command);
        self.record_event("command-queued")
    }

    /// Returns the number of queued commands waiting for a route.
    pub fn queued_command_count(&self) -> usize {
        self.queued_commands.len()
    }

    /// Returns the commands flushed after route reconnects.
    pub fn flushed_commands(&self) -> &[Expr] {
        &self.flushed_commands
    }

    /// Flushes queued actuator commands after a route reconnects.
    pub fn flush_queued_commands(&mut self) -> Result<Vec<Expr>> {
        let flushed = self.queued_commands.drain(..).collect::<Vec<_>>();
        if !flushed.is_empty() {
            self.flushed_commands.extend(flushed.clone());
            self.record_event("command-flushed")?;
        }
        Ok(flushed)
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

    /// Swaps the route and flushes queued commands after the reconnect.
    pub fn rebind_and_flush(&mut self, link: LinkKind) -> Result<Vec<Expr>> {
        self.rebind(link)?;
        self.flush_queued_commands()
    }

    fn record_event(&mut self, name: &str) -> Result<()> {
        self.events.push(
            self.ledger.clone(),
            EventKind::Trace(Ref::Symbol(Symbol::qualified("device/edge", name))),
        )?;
        Ok(())
    }
}

/// Returns the standard preferred route order for watch edge sessions.
pub fn watch_route_order() -> Vec<LinkKind> {
    vec![
        LinkKind::import(),
        LinkKind::modeled(),
        LinkKind::ble_host(),
        LinkKind::relay(),
        LinkKind::zepp_bridge(),
        LinkKind::wifi_if_verified(),
    ]
}

/// Registers a watch profile on a route-transparent edge session.
pub fn register_watch_edge_session(
    id: Symbol,
    profile: DeviceEdgeProfile,
    ledger: LedgerRef,
    initial_link: LinkKind,
) -> Result<DeviceEdgeSession> {
    let mut session = DeviceEdgeSession::with_ledger(id, initial_link, ledger)?;
    session.bind_profile(profile)?;
    session.set_route_order(watch_route_order())?;
    Ok(session)
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

    #[test]
    fn watch_session_flushes_queue_after_reconnect() {
        let id = Symbol::qualified("device/session", "watch-trex3pro-48");
        let ledger = Ref::Symbol(Symbol::qualified("device/ledger", "watch-trex3pro-48"));
        let profile = DeviceEdgeProfile::new(
            Symbol::qualified("device", "amazfit-t-rex-3-pro"),
            Symbol::qualified("device/provider", "watch-wristbridge"),
        );
        let consent = build::map(vec![
            ("kind", build::qsym("device", "consent-receipt")),
            ("session", Expr::Symbol(id.clone())),
            ("seq", build::uint(9)),
        ]);
        let command = build::map(vec![
            ("kind", build::qsym("view-wrist", "command")),
            ("command", build::qsym("watch/command", "notify")),
            ("title", build::text("route")),
        ]);

        let mut session = register_watch_edge_session(
            id.clone(),
            profile.clone(),
            ledger.clone(),
            LinkKind::Direct,
        )
        .unwrap();
        session.bind_consent(consent.clone()).unwrap();
        session.queue_command(command.clone()).unwrap();

        let events_before = session.event_ledger().len_for_run(&ledger);
        let flushed = session.rebind_and_flush(LinkKind::relay()).unwrap();

        assert_eq!(session.id(), &id);
        assert_eq!(session.ledger_ref(), &ledger);
        assert_eq!(session.link(), &LinkKind::Relay);
        assert_eq!(session.device_profile(), Some(&profile));
        assert_eq!(session.bound_consent(), Some(&consent));
        assert_eq!(session.bound_consent_session(), Some(id));
        assert_eq!(session.route_order(), watch_route_order().as_slice());
        assert_eq!(flushed, vec![command.clone()]);
        assert_eq!(session.flushed_commands(), &[command]);
        assert_eq!(session.queued_command_count(), 0);
        assert_eq!(
            session.event_ledger().len_for_run(&ledger),
            events_before + 2
        );
    }
}
