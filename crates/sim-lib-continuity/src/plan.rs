use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};
use std::collections::BTreeSet;

/// Network use admitted by a continuity plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network route may be selected.
    #[default]
    Offline,
    /// Only explicitly listed route families may be selected.
    AllowListed,
}
impl CitizenField for NetworkPolicy {
    fn encode_field(&self) -> Expr {
        Expr::Symbol(Symbol::new(match self {
            Self::Offline => "offline",
            Self::AllowListed => "allow-listed",
        }))
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        match expr {
            Expr::Symbol(v) if v.to_string() == "offline" => Ok(Self::Offline),
            Expr::Symbol(v) if v.to_string() == "allow-listed" => Ok(Self::AllowListed),
            _ => Err(Error::Eval(format!(
                "{field} must be offline or allow-listed"
            ))),
        }
    }
}

/// One uniquely named role and its service closure.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/RoleDemand", version = 1)]
pub struct RoleDemand {
    /// Stable role name.
    pub role: Symbol,
    /// Whether this is the plan's single root.
    pub root: bool,
    /// Required service names.
    pub required_services: Vec<Symbol>,
    /// Ordered fallback role names.
    pub fallbacks: Vec<Symbol>,
}
impl Default for RoleDemand {
    fn default() -> Self {
        Self {
            role: Symbol::new("role"),
            root: false,
            required_services: vec![],
            fallbacks: vec![],
        }
    }
}
impl CitizenField for RoleDemand {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.role.encode_field(),
            self.root.encode_field(),
            self.required_services.encode_field(),
            self.fallbacks.encode_field(),
        ])
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(v) = expr else {
            return Err(Error::Eval(format!("{field} must be a role demand")));
        };
        let [role, root, required, fallbacks] = v.as_slice() else {
            return Err(Error::Eval(format!("{field} role demand has wrong arity")));
        };
        Ok(Self {
            role: Symbol::decode_field_expr(role, field)?,
            root: bool::decode_field_expr(root, field)?,
            required_services: Vec::<Symbol>::decode_field_expr(required, field)?,
            fallbacks: Vec::<Symbol>::decode_field_expr(fallbacks, field)?,
        })
    }
}

/// Version-one policy for a replayable continuity session.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "continuity/Plan", version = 1)]
pub struct ContinuityPlan {
    /// Schema version; currently exactly one.
    pub schema_version: u64,
    /// Stable plan identity.
    pub plan_id: Symbol,
    /// Role demands.
    pub roles: Vec<RoleDemand>,
    /// Services a candidate route must close over.
    pub available_services: Vec<Symbol>,
    /// Maximum logical-time age of a lease.
    pub max_freshness: u64,
    /// Maximum retained turns.
    pub retention_turns: u64,
    /// Disclosure labels admitted to emitted intents.
    pub disclosure: Vec<Symbol>,
    /// Network policy.
    pub network: NetworkPolicy,
    /// Networked route ids admitted when policy is allow-listed.
    pub allowed_network_routes: Vec<Symbol>,
}
impl Default for ContinuityPlan {
    fn default() -> Self {
        Self {
            schema_version: 1,
            plan_id: Symbol::qualified("continuity.plan", "example"),
            roles: vec![RoleDemand {
                role: Symbol::new("root"),
                root: true,
                ..Default::default()
            }],
            available_services: vec![],
            max_freshness: 1,
            retention_turns: 1,
            disclosure: vec![],
            network: NetworkPolicy::Offline,
            allowed_network_routes: vec![],
        }
    }
}
impl ContinuityPlan {
    /// Validates structural and policy closure without consulting a host.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(Error::Eval("unsupported continuity plan version".into()));
        }
        if self.max_freshness == 0 || self.retention_turns == 0 {
            return Err(Error::Eval(
                "freshness and retention bounds must be positive".into(),
            ));
        }
        if self.roles.iter().filter(|r| r.root).count() != 1 {
            return Err(Error::Eval(
                "continuity plan requires exactly one root".into(),
            ));
        }
        let names: BTreeSet<_> = self.roles.iter().map(|r| r.role.to_string()).collect();
        if names.len() != self.roles.len() {
            return Err(Error::Eval("continuity roles must be unique".into()));
        }
        let services: BTreeSet<_> = self
            .available_services
            .iter()
            .map(ToString::to_string)
            .collect();
        if services.len() != self.available_services.len() {
            return Err(Error::Eval("available services must be unique".into()));
        }
        let disclosures: BTreeSet<_> = self.disclosure.iter().map(ToString::to_string).collect();
        if disclosures.len() != self.disclosure.len() {
            return Err(Error::Eval("disclosure labels must be unique".into()));
        }
        let network_routes: BTreeSet<_> = self
            .allowed_network_routes
            .iter()
            .map(ToString::to_string)
            .collect();
        if network_routes.len() != self.allowed_network_routes.len()
            || (matches!(self.network, NetworkPolicy::Offline) && !network_routes.is_empty())
            || (matches!(self.network, NetworkPolicy::AllowListed) && network_routes.is_empty())
        {
            return Err(Error::Eval(
                "network route allow-list does not match network policy".into(),
            ));
        }
        for role in &self.roles {
            let fallbacks: BTreeSet<_> = role.fallbacks.iter().map(ToString::to_string).collect();
            if fallbacks.len() != role.fallbacks.len() || fallbacks.contains(&role.role.to_string())
            {
                return Err(Error::Eval(
                    "fallbacks must be unique and cannot name their own role".into(),
                ));
            }
            if role
                .required_services
                .iter()
                .any(|s| !services.contains(&s.to_string()))
            {
                return Err(Error::Eval("required-service closure is incomplete".into()));
            }
            if role
                .fallbacks
                .iter()
                .any(|f| !names.contains(&f.to_string()))
            {
                return Err(Error::Eval("fallback closure is incomplete".into()));
            }
        }
        Ok(())
    }
}
