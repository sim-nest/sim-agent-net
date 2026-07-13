//! Configuration probe for model provider defaults.

use regex::Regex;
use sim_config::{
    ConfigDir, ConfigLayer, ConfigProbe, ConfigProbeReport, ConfigProbeRequest, ConfigProbeStatus,
    ConfigSource, ProbeMode,
};
use sim_kernel::{Expr, Symbol};

use crate::ModelCard;

const FIXTURE_ECHO_MODEL: &str = "fixture/echo";
const DEFAULT_MODEL_REGEX: &str = r"^(?:fixture/|sim/|gpt-4\.1|o[34]).*";
const MODELED_PROVIDER_REGEX: &str = r"^(?:modeled)$";

const EMITTED_KEYS: [&str; 7] = [
    "model_regex",
    "provider_regex",
    "prefer_local",
    "default_model",
    "openai_key_present",
    "openai_base_present",
    "ollama_host_present",
];

/// Safe provider-presence facts for model-default probing.
///
/// The probe stores only whether an environment-backed provider is present.
/// It never stores or emits raw environment values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentModelProviderPresence {
    /// Whether `OPENAI_API_KEY` is present.
    pub openai_key_present: bool,
    /// Whether `OPENAI_BASE_URL` is present.
    pub openai_base_present: bool,
    /// Whether `OLLAMA_HOST` is present.
    pub ollama_host_present: bool,
}

impl AgentModelProviderPresence {
    /// Captures provider presence from the current process environment.
    pub fn from_current_env() -> Self {
        Self {
            openai_key_present: std::env::var_os("OPENAI_API_KEY").is_some(),
            openai_base_present: std::env::var_os("OPENAI_BASE_URL").is_some(),
            ollama_host_present: std::env::var_os("OLLAMA_HOST").is_some(),
        }
    }
}

/// Returns the stable config library id for model defaults.
pub fn model_defaults_config_lib_symbol() -> Symbol {
    Symbol::qualified("model", "defaults")
}

/// Returns the stable model-default config probe id.
pub fn agent_model_config_probe_symbol() -> Symbol {
    Symbol::qualified("config-probe", "model")
}

/// Safe config probe for AI provider and model defaults.
pub struct AgentModelConfigProbe {
    cards: Vec<ModelCard>,
    model_regex: Regex,
    provider_presence: AgentModelProviderPresence,
    prefer_local: bool,
}

impl AgentModelConfigProbe {
    /// Builds a probe over caller-supplied runner model cards and provider facts.
    pub fn new(
        cards: impl IntoIterator<Item = ModelCard>,
        provider_presence: AgentModelProviderPresence,
    ) -> Self {
        Self {
            cards: cards.into_iter().collect(),
            model_regex: Regex::new(DEFAULT_MODEL_REGEX)
                .expect("default model-default regex is valid"),
            provider_presence,
            prefer_local: true,
        }
    }

    /// Builds a real-mode probe over cards with provider presence read safely.
    pub fn from_current_env(cards: impl IntoIterator<Item = ModelCard>) -> Self {
        Self::new(cards, AgentModelProviderPresence::from_current_env())
    }

    /// Builds the deterministic modeled probe used by default in tests.
    pub fn modeled() -> Self {
        Self::new(Vec::new(), AgentModelProviderPresence::default())
    }
}

impl Default for AgentModelConfigProbe {
    fn default() -> Self {
        Self::modeled()
    }
}

impl ConfigProbe for AgentModelConfigProbe {
    fn symbol(&self) -> Symbol {
        agent_model_config_probe_symbol()
    }

    fn probe(&self, request: &ConfigProbeRequest) -> (Option<ConfigLayer>, ConfigProbeReport) {
        if request.lib != model_defaults_config_lib_symbol() {
            return (
                None,
                report(
                    self.symbol(),
                    request,
                    ConfigProbeStatus::Skipped {
                        reason: "model probe only serves model/defaults".to_owned(),
                    },
                    &[],
                ),
            );
        }

        if request.mode == ProbeMode::Real && !request.caps.env {
            return (
                None,
                report(
                    self.symbol(),
                    request,
                    ConfigProbeStatus::Denied {
                        capability: "env".to_owned(),
                    },
                    &[],
                ),
            );
        }

        let (default_model, provider_regex, provider_presence) = match request.mode {
            ProbeMode::Modeled => (
                FIXTURE_ECHO_MODEL.to_owned(),
                MODELED_PROVIDER_REGEX.to_owned(),
                AgentModelProviderPresence::default(),
            ),
            ProbeMode::Real => (
                self.selected_model(),
                candidate_regex(&self.provider_candidates()),
                self.provider_presence,
            ),
        };

        let layer = ConfigLayer::new(
            ConfigSource::Probe {
                probe: self.symbol(),
                mode: request.mode,
            },
            ConfigDir::one(
                request.lib.clone(),
                model_defaults_table(
                    &default_model,
                    &provider_regex,
                    self.prefer_local,
                    provider_presence,
                ),
            )
            .expect("model-defaults config probe builds a map table"),
        );
        (
            Some(layer),
            report(
                self.symbol(),
                request,
                ConfigProbeStatus::Applied,
                &EMITTED_KEYS,
            ),
        )
    }
}

impl AgentModelConfigProbe {
    fn selected_model(&self) -> String {
        let mut matches = self
            .cards
            .iter()
            .filter(|card| self.model_regex.is_match(&card.model))
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            model_sort_key(left, self.prefer_local).cmp(&model_sort_key(right, self.prefer_local))
        });
        matches
            .first()
            .map(|card| card.model.clone())
            .unwrap_or_else(|| FIXTURE_ECHO_MODEL.to_owned())
    }

    fn provider_candidates(&self) -> Vec<String> {
        let mut candidates = vec!["modeled".to_owned()];
        if self.provider_presence.openai_key_present {
            push_unique(&mut candidates, "openai");
        }
        if self.provider_presence.openai_base_present {
            push_unique(&mut candidates, "openai-compatible");
        }
        if self.provider_presence.ollama_host_present {
            push_unique(&mut candidates, "ollama");
        }
        let mut card_providers = self
            .cards
            .iter()
            .map(|card| card.provider.name.to_string())
            .collect::<Vec<_>>();
        card_providers.sort();
        for provider in card_providers {
            push_unique(&mut candidates, provider);
        }
        candidates
    }
}

fn model_sort_key(card: &ModelCard, prefer_local: bool) -> (u8, &str, &str, &str) {
    let locality = if prefer_local {
        match card.locality.name.as_ref() {
            "local" | "modeled" | "fixture" => 0,
            "fabric" | "agent-backed" => 1,
            "remote" => 2,
            _ => 1,
        }
    } else {
        0
    };
    (
        locality,
        card.model.as_str(),
        card.provider.name.as_ref(),
        card.runner.name.as_ref(),
    )
}

fn model_defaults_table(
    default_model: &str,
    provider_regex: &str,
    prefer_local: bool,
    provider_presence: AgentModelProviderPresence,
) -> Expr {
    Expr::Map(vec![
        (
            key("model_regex"),
            Expr::String(DEFAULT_MODEL_REGEX.to_owned()),
        ),
        (
            key("provider_regex"),
            Expr::String(provider_regex.to_owned()),
        ),
        (key("prefer_local"), Expr::Bool(prefer_local)),
        (key("default_model"), Expr::String(default_model.to_owned())),
        (
            key("openai_key_present"),
            Expr::Bool(provider_presence.openai_key_present),
        ),
        (
            key("openai_base_present"),
            Expr::Bool(provider_presence.openai_base_present),
        ),
        (
            key("ollama_host_present"),
            Expr::Bool(provider_presence.ollama_host_present),
        ),
    ])
}

fn report(
    probe: Symbol,
    request: &ConfigProbeRequest,
    status: ConfigProbeStatus,
    keys: &[&str],
) -> ConfigProbeReport {
    ConfigProbeReport {
        probe,
        lib: request.lib.clone(),
        mode: request.mode,
        status,
        emitted_keys: keys.iter().map(|key| (*key).to_owned()).collect(),
    }
}

fn candidate_regex(values: &[String]) -> String {
    if values.is_empty() {
        return "(?!)".to_owned();
    }
    format!(
        "^(?:{})$",
        values
            .iter()
            .map(|value| regex_escape(value))
            .collect::<Vec<_>>()
            .join("|")
    )
}

fn regex_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn key(name: &str) -> Expr {
    Expr::Symbol(Symbol::new(name))
}

fn push_unique(candidates: &mut Vec<String>, value: impl ToString) {
    let value = value.to_string();
    if !candidates.iter().any(|candidate| candidate == &value) {
        candidates.push(value);
    }
}
