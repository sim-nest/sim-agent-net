//! Private-inventory parsing and redaction-safe provider readiness census.

use sim_kernel::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

const INVENTORY_SCHEMA: &str = "sim.provider-seats/v1";
const FAMILIES: &[&str] = &[
    "anthropic-api",
    "claude-cli",
    "codex-cli",
    "lemonade",
    "lm-studio",
    "ollama",
    "openai-api",
    "opencode-cli",
];

/// Validated, code-free declaration of intended provider seats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderInventory {
    schema: String,
    /// Independently addressable configured seats.
    pub seats: Vec<ProviderSeatConfig>,
}

/// One configured seat. Every string is an opaque, redaction-safe label or reference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderSeatConfig {
    /// Stable inventory id.
    pub id: String,
    /// Registered provider family without the `provider/` prefix.
    pub family: String,
    /// Authentication method (`api-key`, `subscription`, `broker-owned`, or `none`).
    pub auth: String,
    /// Opaque principal registry reference.
    pub principal_ref: String,
    /// Logical endpoint label, never an address containing credentials.
    pub endpoint_label: String,
    /// Opaque config-home registry reference for CLI seats.
    pub config_home_ref: Option<String>,
    /// Secret-provider reference; credential material is forbidden.
    pub secret_source: Option<String>,
    /// Exact acknowledged terms revision, when the family requires one.
    pub terms_acknowledgement: Option<String>,
    /// Optional preferred model default.
    pub preferred_model: Option<String>,
    /// Registered resource-job id used to establish live readiness.
    pub resource_job: Option<String>,
    /// Registered provider-probe id used to establish live readiness.
    pub provider_probe: Option<String>,
}

/// Trusted origin of live readiness evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceSource {
    /// A supervisor-registered resource job.
    RegisteredResourceJob,
    /// A provider adapter's typed probe.
    ProviderProbe,
}

/// Redaction-safe live evidence supplied by a registered boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusEvidence {
    /// Seat inventory id.
    pub seat_id: String,
    /// Registered evidence source.
    pub source: EvidenceSource,
    /// Resulting census state.
    pub state: CensusState,
    /// Redacted reason code or short explanation.
    pub reason: String,
    /// Safe operator action containing no credential or config path.
    pub next_action: String,
}

/// Honest readiness state for one configured seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CensusState {
    /// Typed live evidence says the seat is usable.
    Ready,
    /// Required external service or resource is unavailable.
    Unavailable,
    /// Typed evidence says authentication has expired.
    Expired,
    /// Policy, terms, or schema validation refused the seat.
    Refused,
}

/// One redaction-safe census row, emitted even when no live evidence exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusRow {
    /// Stable inventory id.
    pub seat_id: String,
    /// Registered provider family.
    pub family: String,
    /// Current readiness.
    pub state: CensusState,
    /// Redacted reason.
    pub reason: String,
    /// Next safe operator action.
    pub next_action: String,
}

impl ProviderInventory {
    /// Parses and validates a private provider inventory without consulting host state.
    pub fn from_toml(input: &str) -> Result<Self> {
        reject_secret_shaped_input(input)?;
        let inventory = parse_inventory(input)?;
        inventory.validate()?;
        Ok(inventory)
    }

    /// Validates schema, identities, families, authentication, and evidence declarations.
    pub fn validate(&self) -> Result<()> {
        if self.schema != INVENTORY_SCHEMA {
            return Err(Error::Eval(format!(
                "unsupported provider inventory schema {}",
                self.schema
            )));
        }
        let mut ids = BTreeSet::new();
        for seat in &self.seats {
            for (name, value) in [
                ("seat id", seat.id.as_str()),
                ("principal ref", seat.principal_ref.as_str()),
                ("endpoint label", seat.endpoint_label.as_str()),
            ] {
                validate_safe_label(name, value)?;
            }
            if !ids.insert(&seat.id) {
                return Err(Error::Eval(format!(
                    "duplicate provider seat id {}",
                    seat.id
                )));
            }
            if !FAMILIES.contains(&seat.family.as_str()) {
                return Err(Error::Eval(format!(
                    "unknown provider family {}",
                    seat.family
                )));
            }
            let cli = matches!(
                seat.family.as_str(),
                "codex-cli" | "claude-cli" | "opencode-cli"
            );
            let local = matches!(seat.family.as_str(), "ollama" | "lemonade" | "lm-studio");
            if cli != seat.config_home_ref.is_some() {
                return Err(Error::Eval(format!(
                    "seat {} has an impossible config-home combination",
                    seat.id
                )));
            }
            let secret = seat.secret_source.is_some();
            let valid_auth = match seat.auth.as_str() {
                "api-key" => secret && !local,
                "subscription" => cli && !secret,
                "broker-owned" => cli && !secret,
                "none" => !secret && local,
                _ => false,
            };
            if !valid_auth {
                return Err(Error::Eval(format!(
                    "seat {} has an impossible auth combination",
                    seat.id
                )));
            }
            if seat.resource_job.is_some() == seat.provider_probe.is_some() {
                return Err(Error::Eval(format!(
                    "seat {} must declare exactly one registered readiness source",
                    seat.id
                )));
            }
            for value in [
                seat.config_home_ref.as_deref(),
                seat.secret_source.as_deref(),
                seat.terms_acknowledgement.as_deref(),
                seat.preferred_model.as_deref(),
                seat.resource_job.as_deref(),
                seat.provider_probe.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                validate_safe_label("provider inventory value", value)?;
            }
        }
        Ok(())
    }

    /// Joins typed live evidence and emits exactly one row for every configured seat.
    pub fn census(&self, evidence: &[CensusEvidence]) -> Result<Vec<CensusRow>> {
        let mut live = BTreeMap::new();
        for item in evidence {
            validate_safe_label("census reason", &item.reason)?;
            validate_safe_label("census next action", &item.next_action)?;
            if live.insert(item.seat_id.as_str(), item).is_some() {
                return Err(Error::Eval(format!(
                    "duplicate census evidence for {}",
                    item.seat_id
                )));
            }
        }
        self.seats
            .iter()
            .map(|seat| {
                let item = live.remove(seat.id.as_str());
                if let Some(item) = item {
                    let source_matches =
                        matches!(item.source, EvidenceSource::RegisteredResourceJob)
                            && seat.resource_job.is_some()
                            || matches!(item.source, EvidenceSource::ProviderProbe)
                                && seat.provider_probe.is_some();
                    if !source_matches {
                        return Err(Error::Eval(format!(
                            "unregistered evidence source for {}",
                            seat.id
                        )));
                    }
                    Ok(CensusRow {
                        seat_id: seat.id.clone(),
                        family: seat.family.clone(),
                        state: item.state,
                        reason: item.reason.clone(),
                        next_action: item.next_action.clone(),
                    })
                } else {
                    Ok(CensusRow {
                        seat_id: seat.id.clone(),
                        family: seat.family.clone(),
                        state: CensusState::Unavailable,
                        reason: "no-current-registered-evidence".into(),
                        next_action: "run-declared-readiness-check".into(),
                    })
                }
            })
            .collect()
    }
}

fn parse_inventory(input: &str) -> Result<ProviderInventory> {
    let mut schema = None;
    let mut seats = Vec::new();
    let mut current: Option<ProviderSeatConfig> = None;
    for (index, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[seat]]" {
            if let Some(seat) = current.take() {
                seats.push(seat);
            }
            current = Some(ProviderSeatConfig::default());
            continue;
        }
        let (key, encoded) = line
            .split_once('=')
            .ok_or_else(|| Error::Eval(format!("invalid provider inventory line {}", index + 1)))?;
        let key = key.trim();
        let encoded = encoded.trim();
        let value = encoded
            .strip_prefix('"')
            .and_then(|item| item.strip_suffix('"'))
            .filter(|item| !item.contains('"'))
            .ok_or_else(|| {
                Error::Eval(format!(
                    "provider inventory value on line {} must be a simple string",
                    index + 1
                ))
            })?
            .to_owned();
        if current.is_none() {
            if key != "schema" || schema.replace(value).is_some() {
                return Err(Error::Eval(
                    "provider inventory requires one leading schema".into(),
                ));
            }
            continue;
        }
        let seat = current.as_mut().expect("checked above");
        let target = match key {
            "id" => &mut seat.id,
            "family" => &mut seat.family,
            "auth" => &mut seat.auth,
            "principal_ref" => &mut seat.principal_ref,
            "endpoint_label" => &mut seat.endpoint_label,
            "config_home_ref" => {
                set_optional(&mut seat.config_home_ref, value)?;
                continue;
            }
            "secret_source" => {
                set_optional(&mut seat.secret_source, value)?;
                continue;
            }
            "terms_acknowledgement" => {
                set_optional(&mut seat.terms_acknowledgement, value)?;
                continue;
            }
            "preferred_model" => {
                set_optional(&mut seat.preferred_model, value)?;
                continue;
            }
            "resource_job" => {
                set_optional(&mut seat.resource_job, value)?;
                continue;
            }
            "provider_probe" => {
                set_optional(&mut seat.provider_probe, value)?;
                continue;
            }
            other => {
                return Err(Error::Eval(format!(
                    "unknown provider inventory field {other}"
                )));
            }
        };
        if !target.is_empty() {
            return Err(Error::Eval(format!(
                "duplicate provider inventory field {key}"
            )));
        }
        *target = value;
    }
    if let Some(seat) = current {
        seats.push(seat);
    }
    Ok(ProviderInventory {
        schema: schema.ok_or_else(|| Error::Eval("provider inventory schema is missing".into()))?,
        seats,
    })
}

fn set_optional(slot: &mut Option<String>, value: String) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(Error::Eval(
            "duplicate optional provider inventory field".into(),
        ));
    }
    Ok(())
}

fn validate_safe_label(name: &str, value: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    let unsafe_value = value.is_empty()
        || value.starts_with('/')
        || value.contains("\\\\")
        || lower.contains("bearer ")
        || lower.contains("sk-")
        || lower.contains("token=")
        || lower.contains("api_key=")
        || lower.contains("api-key=")
        || lower.contains("prompt")
        || lower.contains("model-output")
        || lower.contains("completion");
    if unsafe_value {
        Err(Error::Eval(format!(
            "{name} is not a redaction-safe reference"
        )))
    } else {
        Ok(())
    }
}

fn reject_secret_shaped_input(input: &str) -> Result<()> {
    let lower = input.to_ascii_lowercase();
    for marker in [
        "-----begin ",
        "bearer ",
        "sk-",
        "api_key =",
        "api-key =",
        "access_token",
        "refresh_token",
    ] {
        if lower.contains(marker) {
            return Err(Error::Eval(
                "provider inventory contains secret-shaped material".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVENTORY: &str = r#"
schema = "sim.provider-seats/v1"
[[seat]]
id = "openai-api-primary"
family = "openai-api"
auth = "api-key"
principal_ref = "principal/openai-primary"
endpoint_label = "openai-public-api"
secret_source = "secret-provider/openai-primary"
terms_acknowledgement = "openai-api/2026-01"
provider_probe = "probe/openai-api"
[[seat]]
id = "codex-subscription-primary"
family = "codex-cli"
auth = "subscription"
principal_ref = "principal/openai-primary"
endpoint_label = "codex-cli"
config_home_ref = "config-home/codex-primary"
terms_acknowledgement = "codex-cli/2026-01"
provider_probe = "probe/codex-cli-primary"
[[seat]]
id = "ollama-local-primary"
family = "ollama"
auth = "none"
principal_ref = "principal/local-none"
endpoint_label = "ollama-local-primary"
resource_job = "resource/ollama-local-primary-health"
"#;

    #[test]
    fn api_subscription_cli_and_local_daemon_remain_separate_census_rows() {
        let inventory = ProviderInventory::from_toml(INVENTORY).unwrap();
        let rows = inventory
            .census(&[
                CensusEvidence {
                    seat_id: "openai-api-primary".into(),
                    source: EvidenceSource::ProviderProbe,
                    state: CensusState::Ready,
                    reason: "typed-probe-ready".into(),
                    next_action: "none-required".into(),
                },
                CensusEvidence {
                    seat_id: "codex-subscription-primary".into(),
                    source: EvidenceSource::ProviderProbe,
                    state: CensusState::Expired,
                    reason: "typed-session-expired".into(),
                    next_action: "run-provider-login".into(),
                },
            ])
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|row| row.state).collect::<Vec<_>>(),
            vec![
                CensusState::Ready,
                CensusState::Expired,
                CensusState::Unavailable
            ]
        );
        assert_eq!(rows[2].next_action, "run-declared-readiness-check");
    }

    #[test]
    fn malformed_inventory_and_unredacted_evidence_fail_closed() {
        for bad in [
            INVENTORY.replace("openai-api-primary", "codex-subscription-primary"),
            INVENTORY.replace("openai-api\"", "mystery\""),
            INVENTORY.replace("secret-provider/openai-primary", "sk-live-secret"),
            INVENTORY.replace("config-home/codex-primary", "/home/operator/.codex"),
            INVENTORY.replace(
                "schema = \"sim.provider-seats/v1\"",
                "schema = \"sim.provider-seats/v0\"",
            ),
        ] {
            assert!(ProviderInventory::from_toml(&bad).is_err());
        }
        let inventory = ProviderInventory::from_toml(INVENTORY).unwrap();
        let leaked = CensusEvidence {
            seat_id: "openai-api-primary".into(),
            source: EvidenceSource::ProviderProbe,
            state: CensusState::Refused,
            reason: "Bearer secret".into(),
            next_action: "inspect prompt".into(),
        };
        assert!(inventory.census(&[leaked]).is_err());
    }

    #[test]
    fn evidence_must_use_the_seat_declared_registered_boundary() {
        let inventory = ProviderInventory::from_toml(INVENTORY).unwrap();
        let wrong = CensusEvidence {
            seat_id: "ollama-local-primary".into(),
            source: EvidenceSource::ProviderProbe,
            state: CensusState::Ready,
            reason: "exit-code-zero".into(),
            next_action: "none-required".into(),
        };
        assert!(inventory.census(&[wrong]).is_err());
    }
}
