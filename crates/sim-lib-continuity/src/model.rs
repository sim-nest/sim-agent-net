use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// A bounded, logical-time route observation; it carries no authority token.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/RouteLease", version = 1)]
pub struct RouteLease {
    /// Candidate route id.
    pub route: Symbol,
    /// Observed logical time.
    pub observed_at: u64,
    /// Expiry logical time.
    pub expires_at: u64,
    /// Services claimed by the observation.
    pub services: Vec<Symbol>,
    /// Whether the candidate requires network access.
    pub networked: bool,
}
impl Default for RouteLease {
    fn default() -> Self {
        Self {
            route: Symbol::new("route"),
            observed_at: 0,
            expires_at: 0,
            services: vec![],
            networked: false,
        }
    }
}
impl CitizenField for RouteLease {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.route.encode_field(),
            self.observed_at.encode_field(),
            self.expires_at.encode_field(),
            self.services.encode_field(),
            self.networked.encode_field(),
        ])
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(v) = expr else {
            return Err(Error::Eval(format!("{field} must be a route lease")));
        };
        let [route, observed, expires, services, networked] = v.as_slice() else {
            return Err(Error::Eval(format!("{field} route lease has wrong arity")));
        };
        Ok(Self {
            route: Symbol::decode_field_expr(route, field)?,
            observed_at: u64::decode_field_expr(observed, field)?,
            expires_at: u64::decode_field_expr(expires, field)?,
            services: Vec::<Symbol>::decode_field_expr(services, field)?,
            networked: bool::decode_field_expr(networked, field)?,
        })
    }
}

/// An authored or observed input to the pure reducer.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/Event", version = 1)]
pub struct ContinuityEvent {
    /// Stable id used for deduplication.
    pub event_id: Symbol,
    /// Monotonic per-plan sequence.
    pub sequence: u64,
    /// Caller-supplied logical time.
    pub logical_time: u64,
    /// Open event kind.
    pub kind: Symbol,
    /// Target role.
    pub role: Symbol,
    /// Optional candidate route.
    pub lease: Option<RouteLease>,
    /// Already-redacted payload.
    pub payload: Expr,
    /// Disclosure class of the payload, when it is not public.
    pub disclosure: Option<Symbol>,
}
impl Default for ContinuityEvent {
    fn default() -> Self {
        Self {
            event_id: Symbol::qualified("event", "example"),
            sequence: 0,
            logical_time: 0,
            kind: Symbol::new("observed"),
            role: Symbol::new("root"),
            lease: None,
            payload: Expr::Nil,
            disclosure: None,
        }
    }
}
impl CitizenField for ContinuityEvent {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.event_id.encode_field(),
            self.sequence.encode_field(),
            self.logical_time.encode_field(),
            self.kind.encode_field(),
            self.role.encode_field(),
            self.lease.encode_field(),
            self.payload.clone(),
            self.disclosure.encode_field(),
        ])
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(v) = expr else {
            return Err(Error::Eval(format!("{field} must be an event")));
        };
        let [id, sequence, time, kind, role, lease, payload, disclosure] = v.as_slice() else {
            return Err(Error::Eval(format!("{field} event has wrong arity")));
        };
        Ok(Self {
            event_id: Symbol::decode_field_expr(id, field)?,
            sequence: u64::decode_field_expr(sequence, field)?,
            logical_time: u64::decode_field_expr(time, field)?,
            kind: Symbol::decode_field_expr(kind, field)?,
            role: Symbol::decode_field_expr(role, field)?,
            lease: Option::<RouteLease>::decode_field_expr(lease, field)?,
            payload: payload.clone(),
            disclosure: Option::<Symbol>::decode_field_expr(disclosure, field)?,
        })
    }
}

/// Effect request emitted as data for an authorized host.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/Intent", version = 1)]
pub struct ContinuityIntent {
    /// Intent kind.
    pub kind: Symbol,
    /// Role concerned.
    pub role: Symbol,
    /// Candidate route, never authority.
    pub route: Option<Symbol>,
    /// Redacted detail.
    pub detail: Expr,
}
impl Default for ContinuityIntent {
    fn default() -> Self {
        Self {
            kind: Symbol::new("record"),
            role: Symbol::new("root"),
            route: None,
            detail: Expr::Nil,
        }
    }
}
impl CitizenField for ContinuityIntent {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.kind.encode_field(),
            self.role.encode_field(),
            self.route.encode_field(),
            self.detail.clone(),
        ])
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(v) = expr else {
            return Err(Error::Eval(format!("{field} must be an intent")));
        };
        let [kind, role, route, detail] = v.as_slice() else {
            return Err(Error::Eval(format!("{field} intent has wrong arity")));
        };
        Ok(Self {
            kind: Symbol::decode_field_expr(kind, field)?,
            role: Symbol::decode_field_expr(role, field)?,
            route: Option::<Symbol>::decode_field_expr(route, field)?,
            detail: detail.clone(),
        })
    }
}

/// Typed rejection that leaves state unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/Refusal", version = 1)]
pub struct ContinuityRefusal {
    /// Stable refusal code.
    pub code: Symbol,
    /// Redacted diagnostic.
    pub detail: Expr,
}
impl Default for ContinuityRefusal {
    fn default() -> Self {
        Self {
            code: Symbol::new("refused"),
            detail: Expr::Nil,
        }
    }
}

/// Canonical accepted transition.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/Turn", version = 1)]
pub struct ContinuityTurn {
    /// Event sequence.
    pub sequence: u64,
    /// Event id.
    pub event_id: Symbol,
    /// Logical time.
    pub logical_time: u64,
    /// Input event.
    pub event: ContinuityEvent,
    /// Deterministically emitted intents.
    pub intents: Vec<ContinuityIntent>,
}
impl Default for ContinuityTurn {
    fn default() -> Self {
        let event = ContinuityEvent::default();
        Self {
            sequence: 0,
            event_id: event.event_id.clone(),
            logical_time: 0,
            event,
            intents: vec![],
        }
    }
}
