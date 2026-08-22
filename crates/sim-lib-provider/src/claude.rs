use crate::{
    AuthMetadata, AuthMethod, AuthOwner, EndpointCard, HarnessCard, PrincipalCard,
    ProviderFamilyCard, ProviderSeatCard, ProviderSeatId, ProviderSeatLimits, SessionStatus,
    TermsAcknowledgement, provider_operation,
};
use sim_kernel::{Error, Expr, Result, Symbol};

/// Local policy governing whether a Claude subscription may be used through the CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCliTermsPolicy {
    /// Whether the provider's subscription terms permit this use path.
    pub subscription_use_allowed: bool,
    /// Stable provider terms identifier.
    pub terms_id: String,
    /// Exact terms revision evaluated by the operator.
    pub revision: String,
    /// Exact acknowledgement when the use path is permitted.
    pub acknowledgement: Option<TermsAcknowledgement>,
}

impl ClaudeCliTermsPolicy {
    /// Refuses forbidden use and requires an exact acknowledgement otherwise.
    pub fn enforce(&self) -> Result<()> {
        if !self.subscription_use_allowed {
            return Err(Error::Eval(
                "Claude CLI subscription use is forbidden by provider terms policy".into(),
            ));
        }
        self.auth_metadata(SessionStatus::LoginRequired)
            .require_terms()
    }

    /// Produces redaction-safe metadata for a current session.
    pub fn auth_metadata(&self, session: SessionStatus) -> AuthMetadata {
        AuthMetadata {
            owner: AuthOwner::Broker,
            session,
            required_terms: Some((self.terms_id.clone(), self.revision.clone())),
            acknowledgement: self.acknowledgement.clone(),
        }
    }
}

/// Host-supplied, opaque description of one independently authenticated Claude config home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCliConfigHome {
    /// Stable public label used in the seat id.
    pub label: String,
    /// Opaque private-artifact reference for the Claude config directory.
    pub artifact: String,
    /// Exact executable version admitted for this seat.
    pub expected_version: String,
    /// Digest of the config-home identity, without revealing its path.
    pub config_home_digest: String,
    /// Digest of visible, non-secret effective settings.
    pub visible_settings_digest: String,
    /// Model selected for this home.
    pub model: String,
    /// Claude permission mode used for non-interactive tasks.
    pub permission_mode: String,
    /// Maximum agent turns admitted for one request.
    pub max_turns: u32,
    /// Optional maximum spend in US dollars admitted for one request.
    pub max_budget_usd: Option<String>,
    /// Provider-terms decision evaluated before any process connection.
    pub terms_policy: ClaudeCliTermsPolicy,
}

/// Observed compatibility and session facts returned by exact-argv Claude CLI probing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCliProbe {
    /// Exact executable version string.
    pub version: String,
    /// Supported non-interactive machine mode.
    pub machine_mode: String,
    /// Machine-readable output schema.
    pub output_schema: String,
    /// Current broker-owned session state.
    pub session: SessionStatus,
    /// Redaction-safe principal label, when authenticated.
    pub principal_label: Option<String>,
}

/// Returns the canonical `provider/claude-cli` family card.
pub fn claude_cli_family() -> ProviderFamilyCard {
    ProviderFamilyCard {
        family: Symbol::qualified("provider", "claude-cli"),
        transport: Symbol::new("broker-process"),
        semantics: Symbol::new("agent-task"),
        auth_owner: Symbol::new("claude-cli"),
        wires: vec![Symbol::new("claude-stream-json")],
        operations: provider_operation::all(),
        revision: Expr::String("claude-cli-profile/1".into()),
        extra: Vec::new(),
    }
}

impl ClaudeCliConfigHome {
    /// Materializes this config home as a distinct, fully identified provider seat.
    pub fn seat_card(&self, probe: &ClaudeCliProbe) -> Result<ProviderSeatCard> {
        let family = Symbol::qualified("provider", "claude-cli");
        let field =
            |name: &str, value: String| (Expr::Symbol(Symbol::new(name)), Expr::String(value));
        let mut card = ProviderSeatCard {
            seat: ProviderSeatId::new(family.clone(), &self.label)?,
            family,
            principal: PrincipalCard {
                label: probe
                    .principal_label
                    .clone()
                    .unwrap_or_else(|| "login-required".into()),
                kind: if matches!(probe.session, SessionStatus::Authenticated { .. }) {
                    AuthMethod::Subscription.symbol()
                } else {
                    AuthMethod::BrokerOwned.symbol()
                },
                source: Symbol::new("claude-cli"),
                digest: self.config_home_digest.clone(),
                extra: Vec::new(),
            },
            endpoint: EndpointCard {
                address: self.label.clone(),
                transport: Symbol::new("local-process"),
                revision: Expr::String(probe.output_schema.clone()),
                extra: Vec::new(),
            },
            harness: Some(HarnessCard {
                kind: Symbol::new("vendor-cli"),
                label: "claude".into(),
                revision: Expr::String(probe.version.clone()),
                extra: vec![
                    field("machine-mode", probe.machine_mode.clone()),
                    field("config-home-digest", self.config_home_digest.clone()),
                    field(
                        "visible-settings-digest",
                        self.visible_settings_digest.clone(),
                    ),
                    field("selected-model", self.model.clone()),
                    field("permission-mode", self.permission_mode.clone()),
                    field("max-turns", self.max_turns.to_string()),
                    field(
                        "max-budget-usd",
                        self.max_budget_usd.clone().unwrap_or_else(|| "none".into()),
                    ),
                ],
            }),
            model: Some(self.model.clone()),
            limits: ProviderSeatLimits::default(),
            revision: Expr::String("claude-seat/1".into()),
            extra: vec![field("semantics", "agent-task".into())],
        };
        card.set_auth_metadata(&self.terms_policy.auth_metadata(probe.session.clone()));
        Ok(card)
    }
}
