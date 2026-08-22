use crate::{
    AuthMethod, EndpointCard, HarnessCard, PrincipalCard, ProviderFamilyCard, ProviderSeatCard,
    ProviderSeatId, ProviderSeatLimits, provider_operation,
};
use sim_kernel::{Expr, Result, Symbol};

/// Host-supplied, opaque description of one independently authenticated Codex CLI home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexCliConfigHome {
    /// Stable public label used in the seat id.
    pub label: String,
    /// Opaque private-artifact reference for `CODEX_HOME`.
    pub artifact: String,
    /// Exact executable revision admitted for this seat.
    pub expected_version: String,
    /// Digest of the effective, redaction-safe configuration.
    pub config_digest: String,
    /// Model selected for this home.
    pub model: String,
    /// Exact Codex sandbox mode.
    pub sandbox_mode: String,
    /// Workspace posture supplied to `codex exec`.
    pub workspace_posture: String,
    /// Digest of declared plugin configuration, or `none`.
    pub plugin_digest: String,
    /// Digest of the declared approval mode.
    pub approval_digest: String,
}

/// Observed compatibility facts returned by exact-argv Codex CLI probing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexCliProbe {
    /// Exact executable version string.
    pub version: String,
    /// Supported non-interactive mode.
    pub machine_mode: String,
    /// Authentication method currently visible to the CLI.
    pub auth_method: AuthMethod,
    /// Machine-readable output schema.
    pub output_schema: String,
    /// Redacted principal label, when authenticated.
    pub principal_label: Option<String>,
}

/// Returns the canonical `provider/codex-cli` family card.
pub fn codex_cli_family() -> ProviderFamilyCard {
    ProviderFamilyCard {
        family: Symbol::qualified("provider", "codex-cli"),
        transport: Symbol::new("broker-process"),
        semantics: Symbol::new("agent-task"),
        auth_owner: Symbol::new("codex-cli"),
        wires: vec![Symbol::new("codex-jsonl")],
        operations: provider_operation::all(),
        revision: Expr::String("codex-cli-profile/1".into()),
        extra: Vec::new(),
    }
}

impl CodexCliConfigHome {
    /// Materializes this configured home as a distinct, fully identified provider seat.
    pub fn seat_card(&self, probe: &CodexCliProbe) -> Result<ProviderSeatCard> {
        let family = Symbol::qualified("provider", "codex-cli");
        let field =
            |name: &str, value: String| (Expr::Symbol(Symbol::new(name)), Expr::String(value));
        Ok(ProviderSeatCard {
            seat: ProviderSeatId::new(family.clone(), &self.label)?,
            family,
            principal: PrincipalCard {
                label: probe
                    .principal_label
                    .clone()
                    .unwrap_or_else(|| "login-required".into()),
                kind: probe.auth_method.symbol(),
                source: Symbol::new("codex-cli"),
                digest: self.config_digest.clone(),
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
                label: "codex".into(),
                revision: Expr::String(probe.version.clone()),
                extra: vec![
                    field("machine-mode", probe.machine_mode.clone()),
                    field("config-digest", self.config_digest.clone()),
                    field("sandbox-mode", self.sandbox_mode.clone()),
                    field("workspace-posture", self.workspace_posture.clone()),
                    field("plugin-digest", self.plugin_digest.clone()),
                    field("approval-digest", self.approval_digest.clone()),
                ],
            }),
            model: Some(self.model.clone()),
            limits: ProviderSeatLimits::default(),
            revision: Expr::String("codex-seat/1".into()),
            extra: vec![field("semantics", "agent-task".into())],
        })
    }
}
