use crate::{
    AuthMetadata, AuthMethod, AuthOwner, EndpointCard, HarnessCard, PrincipalCard,
    ProviderFamilyCard, ProviderSeatCard, ProviderSeatId, ProviderSeatLimits, SessionStatus,
    TermsAcknowledgement, provider_operation,
};
use sim_kernel::{Error, Expr, Result, Symbol};

/// The one explicitly selected OpenCode transport for a seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenCodeTransport {
    /// Invoke the configured executable for every inference.
    Process,
    /// Connect to one already-running, explicitly declared local server.
    LocalServer {
        /// Explicit endpoint; discovery never scans or starts a server.
        endpoint: String,
        /// Opaque password reference resolved only by the server transport.
        password_ref: String,
    },
}

impl OpenCodeTransport {
    /// Stable transport identity recorded on family and seat cards.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Process => "local-process",
            Self::LocalServer { .. } => "local-server",
        }
    }
}

/// Policy for a vendor credential observed behind OpenCode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCodeTermsPolicy {
    /// Vendor selected in OpenCode, independently of OpenCode itself.
    pub vendor: String,
    /// Credential class observed without reading credential material.
    pub credential_kind: AuthMethod,
    /// Whether this vendor permits the credential through third-party harnesses.
    pub use_allowed: bool,
    /// Stable terms identifier and revision evaluated by the operator.
    pub terms_id: String,
    /// Exact terms revision evaluated by the operator.
    pub revision: String,
    /// Exact acknowledgement, when use is permitted.
    pub acknowledgement: Option<TermsAcknowledgement>,
}

impl OpenCodeTermsPolicy {
    /// Enforces policy before any OpenCode transport is contacted.
    pub fn enforce(&self) -> Result<()> {
        if !self.use_allowed {
            return Err(Error::Eval(format!(
                "{} {:?} credential is forbidden through OpenCode by provider terms policy",
                self.vendor, self.credential_kind
            )));
        }
        self.metadata(SessionStatus::LoginRequired).require_terms()
    }

    pub(crate) fn metadata(&self, session: SessionStatus) -> AuthMetadata {
        AuthMetadata {
            owner: AuthOwner::Broker,
            session,
            required_terms: Some((self.terms_id.clone(), self.revision.clone())),
            acknowledgement: self.acknowledgement.clone(),
        }
    }
}

/// Host-declared configuration for one OpenCode seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCodeConfig {
    /// Stable public seat label.
    pub label: String,
    /// Opaque config-home artifact; never interpreted by SIM.
    pub artifact: String,
    /// Exact executable or server revision admitted for this seat.
    pub expected_version: String,
    /// Opaque project-root reference used by process execution.
    pub workspace: String,
    /// Explicit OpenCode provider selection.
    pub provider: String,
    /// Explicit OpenCode model selection.
    pub model: String,
    /// Explicit OpenCode agent selection.
    pub agent: String,
    /// Digest of effective redaction-safe configuration.
    pub config_digest: String,
    /// Digest of the declared plugin set.
    pub plugin_digest: String,
    /// One fixed transport; adapters never fall back to another kind.
    pub transport: OpenCodeTransport,
    /// Provider terms decision enforced before transport.
    pub terms_policy: OpenCodeTermsPolicy,
}

/// Redaction-safe facts observed from the configured OpenCode harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCodeProbe {
    /// Observed executable or server revision.
    pub version: String,
    /// Observed machine-readable event schema.
    pub output_schema: String,
    /// Redaction-safe observed session state.
    pub session: SessionStatus,
    /// Digest of the observed catalog only; the catalog is never registered as SIM authority.
    pub observed_catalog_digest: String,
}

/// Canonical `provider/opencode-cli` family card.
pub fn opencode_cli_family() -> ProviderFamilyCard {
    ProviderFamilyCard {
        family: Symbol::qualified("provider", "opencode-cli"),
        transport: Symbol::new("broker-declared"),
        semantics: Symbol::new("agent-task"),
        auth_owner: Symbol::new("opencode-cli"),
        wires: vec![Symbol::new("opencode-json-events")],
        operations: provider_operation::all(),
        revision: Expr::String("opencode-cli-profile/1".into()),
        extra: Vec::new(),
    }
}

impl OpenCodeConfig {
    /// Materializes one declaration as one seat without ambient discovery.
    pub fn seat_card(&self, probe: &OpenCodeProbe) -> Result<ProviderSeatCard> {
        let family = Symbol::qualified("provider", "opencode-cli");
        let field =
            |name: &str, value: String| (Expr::Symbol(Symbol::new(name)), Expr::String(value));
        let address = match &self.transport {
            OpenCodeTransport::Process => self.label.clone(),
            OpenCodeTransport::LocalServer { endpoint, .. } => endpoint.clone(),
        };
        let mut card = ProviderSeatCard {
            seat: ProviderSeatId::new(family.clone(), &self.label)?,
            family,
            principal: PrincipalCard {
                label: self.provider.clone(),
                kind: self.terms_policy.credential_kind.symbol(),
                source: Symbol::new("opencode-cli"),
                digest: self.config_digest.clone(),
                extra: Vec::new(),
            },
            endpoint: EndpointCard {
                address,
                transport: Symbol::new(self.transport.name()),
                revision: Expr::String(probe.output_schema.clone()),
                extra: Vec::new(),
            },
            harness: Some(HarnessCard {
                kind: Symbol::new("vendor-harness"),
                label: "opencode".into(),
                revision: Expr::String(probe.version.clone()),
                extra: vec![
                    field("provider-selection", self.provider.clone()),
                    field("model-selection", self.model.clone()),
                    field("agent-selection", self.agent.clone()),
                    field("config-digest", self.config_digest.clone()),
                    field("plugin-digest", self.plugin_digest.clone()),
                    field("transport-kind", self.transport.name().into()),
                    field(
                        "observed-catalog-digest",
                        probe.observed_catalog_digest.clone(),
                    ),
                ],
            }),
            model: Some(self.model.clone()),
            limits: ProviderSeatLimits::default(),
            revision: Expr::String("opencode-seat/1".into()),
            extra: vec![field("catalog-authority", "observed-only".into())],
        };
        card.set_auth_metadata(&self.terms_policy.metadata(probe.session.clone()));
        Ok(card)
    }
}
