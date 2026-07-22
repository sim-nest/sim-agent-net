//! Provider health probing for HTTP model profiles.

mod transport;

use crate::{ProviderAuth, ProviderConfig, redact::redact_text};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::ModelCard;
use sim_lib_net_core::{HttpHead, UrlParts, parse_url};

pub use transport::HttpProbeTransport;

/// Provider health state reported by a probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeStatus {
    /// The provider answered and model metadata was decoded.
    Available,
    /// The provider was reached but returned an unusable response, or transport failed.
    Unavailable,
    /// The probe was intentionally not sent, such as when a required secret is absent.
    Skipped,
}

impl ProbeStatus {
    /// Symbol form used when exposing the report as expression data.
    pub fn as_symbol(&self) -> Symbol {
        match self {
            Self::Available => Symbol::new("available"),
            Self::Unavailable => Symbol::new("unavailable"),
            Self::Skipped => Symbol::new("skipped"),
        }
    }
}

/// Provider health report returned by [`probe_provider`].
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderProbeReport {
    /// Provider profile id.
    pub provider: Symbol,
    /// Endpoint that was probed.
    pub endpoint: String,
    /// Probe outcome.
    pub status: ProbeStatus,
    /// Models discovered from the provider model-list response.
    pub models: Vec<String>,
    /// Model cards derived from discovered provider metadata.
    pub model_cards: Vec<ModelCard>,
    /// Redacted failure or skip reason.
    pub reason: Option<String>,
    /// Whether the profile has an authentication shape whose secret is redacted.
    pub redacted: bool,
}

impl ProviderProbeReport {
    /// Converts this report to table-visible expression data.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            map_entry("provider", Expr::Symbol(self.provider.clone())),
            map_entry("endpoint", Expr::String(self.endpoint.clone())),
            map_entry("status", Expr::Symbol(self.status.as_symbol())),
            map_entry(
                "models",
                Expr::List(self.models.iter().cloned().map(Expr::String).collect()),
            ),
            map_entry(
                "cards",
                Expr::List(self.model_cards.iter().cloned().map(Expr::from).collect()),
            ),
            map_entry(
                "reason",
                self.reason
                    .as_ref()
                    .map(|reason| Expr::String(reason.clone()))
                    .unwrap_or(Expr::Nil),
            ),
            map_entry("redacted", Expr::Bool(self.redacted)),
        ])
    }
}

/// Candidate endpoint and model-list path tried by provider probing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointCandidate {
    /// Candidate base endpoint.
    pub endpoint: String,
    /// Provider-specific model-list path for this candidate.
    pub models_path: &'static str,
}

/// HTTP GET request issued by a provider probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeHttpRequest<'a> {
    /// Base endpoint string from the provider config.
    pub endpoint: &'a str,
    /// Parsed base endpoint.
    pub endpoint_parts: UrlParts,
    /// Provider-specific model-list path.
    pub path: &'a str,
    /// Provider-specific headers, including authentication when configured.
    pub headers: Vec<(String, String)>,
    /// Socket read/write timeout.
    pub timeout: std::time::Duration,
    /// Maximum response body bytes to decode.
    pub max_response_bytes: usize,
}

/// HTTP response returned by a provider probe transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeHttpResponse {
    /// Parsed HTTP response head.
    pub head: HttpHead,
    /// Decoded response body bytes.
    pub body: Vec<u8>,
}

impl ProbeHttpResponse {
    /// Numeric HTTP status code.
    pub fn status(&self) -> u16 {
        self.head.status
    }
}

/// Transport used by provider probes.
pub trait ProbeTransport {
    /// Sends the model-list GET request.
    fn get(&self, request: ProbeHttpRequest<'_>) -> Result<ProbeHttpResponse>;
}

/// Probes the configured provider's model-list endpoint.
pub fn probe_provider(
    transport: &dyn ProbeTransport,
    config: &ProviderConfig,
) -> Result<ProviderProbeReport> {
    let redacted = auth_is_redacted(&config.profile.auth, config.api_key_env.as_deref());
    let mut headers = Vec::new();
    let mut secrets = Vec::new();
    match auth_header(&config.profile.auth, config.api_key_env.as_deref())? {
        AuthHeader::Header {
            name,
            value,
            secret,
        } => {
            secrets.push(secret);
            secrets.push(value.clone());
            headers.push((name, value));
        }
        AuthHeader::MissingSecret { env } => {
            let mut report = empty_report(config, &config.endpoint, redacted);
            report.status = ProbeStatus::Skipped;
            report.reason = Some(format!("missing secret env {env}"));
            return Ok(report);
        }
        AuthHeader::NoHeader => {}
    }
    if config.profile.provider == Symbol::new("anthropic") {
        headers.push(("anthropic-version".to_owned(), "2023-06-01".to_owned()));
    }

    let mut last_report = None;
    for candidate in endpoint_candidates(config) {
        let report = probe_candidate(
            transport,
            config,
            &candidate,
            headers.clone(),
            &secrets,
            redacted,
        )?;
        if report.status == ProbeStatus::Available {
            return Ok(report);
        }
        last_report = Some(report);
    }
    Ok(last_report.unwrap_or_else(|| empty_report(config, &config.endpoint, redacted)))
}

fn probe_candidate(
    transport: &dyn ProbeTransport,
    config: &ProviderConfig,
    candidate: &EndpointCandidate,
    headers: Vec<(String, String)>,
    secrets: &[String],
    redacted: bool,
) -> Result<ProviderProbeReport> {
    let mut report = empty_report(config, &candidate.endpoint, redacted);
    let endpoint_parts = parse_url(&candidate.endpoint)
        .map_err(|error| Error::Eval(format!("invalid provider endpoint: {error}")))?;
    let request = ProbeHttpRequest {
        endpoint: &candidate.endpoint,
        endpoint_parts,
        path: candidate.models_path,
        headers,
        timeout: config.timeout,
        max_response_bytes: config.max_output_bytes,
    };
    let response = match transport.get(request) {
        Ok(response) => response,
        Err(error) => {
            report.reason = Some(redact_error(error, secrets));
            return Ok(report);
        }
    };
    if !(200..300).contains(&response.status()) {
        report.reason = Some(format!(
            "provider probe returned HTTP {}",
            response.status()
        ));
        return Ok(report);
    }
    match parse_provider_model_entries(&config.profile.provider, &response.body) {
        Ok(entries) => {
            report.status = ProbeStatus::Available;
            report.models = entries.iter().map(|entry| entry.name.clone()).collect();
            report.model_cards = provider_model_cards(config, &entries);
        }
        Err(reason) => {
            report.reason = Some(reason);
        }
    }
    Ok(report)
}

fn empty_report(config: &ProviderConfig, endpoint: &str, redacted: bool) -> ProviderProbeReport {
    ProviderProbeReport {
        provider: config.profile.provider.clone(),
        endpoint: endpoint.to_owned(),
        status: ProbeStatus::Unavailable,
        models: Vec::new(),
        model_cards: Vec::new(),
        reason: None,
        redacted,
    }
}

fn endpoint_candidates(config: &ProviderConfig) -> Vec<EndpointCandidate> {
    if config.profile.provider == Symbol::new("lemonade") {
        let configured =
            (config.endpoint != config.profile.default_endpoint).then(|| config.endpoint.clone());
        return lemonade_candidates(configured);
    }
    vec![EndpointCandidate {
        endpoint: config.endpoint.clone(),
        models_path: config.profile.models_path,
    }]
}

/// Returns Lemonade model-list endpoint candidates in probe order.
pub fn lemonade_candidates(configured: Option<String>) -> Vec<EndpointCandidate> {
    if let Some(endpoint) = configured {
        return vec![EndpointCandidate {
            endpoint,
            models_path: "/models",
        }];
    }
    vec![
        EndpointCandidate {
            endpoint: "http://127.0.0.1:13305/v1".to_owned(),
            models_path: "/models",
        },
        EndpointCandidate {
            endpoint: "http://127.0.0.1:13305/api/v1".to_owned(),
            models_path: "/models",
        },
    ]
}

enum AuthHeader {
    Header {
        name: String,
        value: String,
        secret: String,
    },
    MissingSecret {
        env: String,
    },
    NoHeader,
}

fn auth_header(auth: &ProviderAuth, api_key_env: Option<&str>) -> Result<AuthHeader> {
    match (auth, api_key_env) {
        (ProviderAuth::None, _) | (_, None) => Ok(AuthHeader::NoHeader),
        (ProviderAuth::BearerEnv { .. } | ProviderAuth::OptionalBearerEnv { .. }, Some(env)) => {
            match secret_from_env(env) {
                Some(secret) => Ok(AuthHeader::Header {
                    name: "Authorization".to_owned(),
                    value: format!("Bearer {secret}"),
                    secret,
                }),
                None => Ok(AuthHeader::MissingSecret {
                    env: env.to_owned(),
                }),
            }
        }
        (ProviderAuth::HeaderEnv { header, .. }, Some(env)) => match secret_from_env(env) {
            Some(secret) => Ok(AuthHeader::Header {
                name: header.clone(),
                value: secret.clone(),
                secret,
            }),
            None => Ok(AuthHeader::MissingSecret {
                env: env.to_owned(),
            }),
        },
    }
}

fn auth_is_redacted(auth: &ProviderAuth, api_key_env: Option<&str>) -> bool {
    match auth {
        ProviderAuth::None => false,
        ProviderAuth::BearerEnv { .. } | ProviderAuth::HeaderEnv { .. } => api_key_env.is_some(),
        ProviderAuth::OptionalBearerEnv { .. } => api_key_env.is_some(),
    }
}

fn secret_from_env(env: &str) -> Option<String> {
    match std::env::var(env) {
        Ok(secret) if !secret.is_empty() => Some(secret),
        _ => None,
    }
}

/// Parses an Ollama `/api/tags` response into the provider model names it
/// advertises.
pub fn parse_ollama_tags(body: &[u8]) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| Error::Eval(format!("ollama tags invalid json: {error}")))?;
    parse_ollama_models(&value).map_err(Error::Eval)
}

#[derive(Clone, Debug, PartialEq)]
struct ProviderModelEntry {
    name: String,
    extra: Vec<(Expr, Expr)>,
}

fn parse_provider_model_entries(
    provider: &Symbol,
    body: &[u8],
) -> std::result::Result<Vec<ProviderModelEntry>, String> {
    if provider == &Symbol::new("ollama") {
        return parse_ollama_tags(body)
            .map(|models| {
                models
                    .into_iter()
                    .map(|name| ProviderModelEntry {
                        name,
                        extra: Vec::new(),
                    })
                    .collect()
            })
            .map_err(|error| error.to_string());
    }
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| format!("malformed model list json: {error}"))?;
    parse_openai_style_model_entries(provider, &value)
}

fn parse_openai_style_model_entries(
    provider: &Symbol,
    value: &serde_json::Value,
) -> std::result::Result<Vec<ProviderModelEntry>, String> {
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "model list json missing data array".to_owned())?;
    Ok(data
        .iter()
        .filter_map(|item| {
            let name = item.get("id").and_then(serde_json::Value::as_str)?;
            let extra = if provider == &Symbol::new("lemonade") {
                lemonade_model_extras(item)
            } else {
                Vec::new()
            };
            Some(ProviderModelEntry {
                name: name.to_owned(),
                extra,
            })
        })
        .collect())
}

fn parse_ollama_models(value: &serde_json::Value) -> std::result::Result<Vec<String>, String> {
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "ollama model list json missing models array".to_owned())?;
    Ok(models
        .iter()
        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect())
}

fn redact_error(error: Error, secrets: &[String]) -> String {
    let text = error.to_string();
    let secret_refs = secrets.iter().map(String::as_str).collect::<Vec<_>>();
    redact_text(&text, &secret_refs)
}

fn provider_model_cards(config: &ProviderConfig, entries: &[ProviderModelEntry]) -> Vec<ModelCard> {
    entries
        .iter()
        .map(|entry| {
            let mut card = ModelCard::new(
                config.runner.clone(),
                entry.name.clone(),
                config.profile.provider.clone(),
                config.locality.clone(),
            );
            card.extra.extend(entry.extra.clone());
            card
        })
        .collect()
}

fn lemonade_model_extras(item: &serde_json::Value) -> Vec<(Expr, Expr)> {
    let mut extras = Vec::new();
    push_modality_extra(
        &mut extras,
        "modalities",
        modality_field(
            item,
            &["modalities", "modality", "capabilities"],
            &["modalities"],
        ),
    );
    push_modality_extra(
        &mut extras,
        "modalities-in",
        modality_field(
            item,
            &[
                "modalities-in",
                "modalities_in",
                "input-modalities",
                "input_modalities",
            ],
            &["modalities_in", "input_modalities", "input"],
        ),
    );
    push_modality_extra(
        &mut extras,
        "modalities-out",
        modality_field(
            item,
            &[
                "modalities-out",
                "modalities_out",
                "output-modalities",
                "output_modalities",
            ],
            &["modalities_out", "output_modalities", "output"],
        ),
    );
    extras
}

fn modality_field(
    item: &serde_json::Value,
    direct_names: &[&str],
    capability_names: &[&str],
) -> Vec<Symbol> {
    for name in direct_names {
        if let Some(value) = item.get(*name) {
            let symbols = modality_symbols(value);
            if !symbols.is_empty() {
                return symbols;
            }
        }
    }
    let Some(capabilities) = item.get("capabilities") else {
        return Vec::new();
    };
    for name in capability_names {
        if let Some(value) = capabilities.get(*name) {
            let symbols = modality_symbols(value);
            if !symbols.is_empty() {
                return symbols;
            }
        }
    }
    Vec::new()
}

fn modality_symbols(value: &serde_json::Value) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    match value {
        serde_json::Value::String(text) => push_modality_symbol(&mut symbols, text),
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(text) = item.as_str() {
                    push_modality_symbol(&mut symbols, text);
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if value.as_bool().unwrap_or(false) {
                    push_modality_symbol(&mut symbols, key);
                }
            }
        }
        _ => {}
    }
    symbols
}

fn push_modality_symbol(symbols: &mut Vec<Symbol>, text: &str) {
    let name = text.trim().to_ascii_lowercase().replace('_', "-");
    if !matches!(name.as_str(), "text" | "image" | "audio") {
        return;
    }
    let symbol = Symbol::new(name);
    if !symbols.contains(&symbol) {
        symbols.push(symbol);
    }
}

fn push_modality_extra(extras: &mut Vec<(Expr, Expr)>, name: &str, symbols: Vec<Symbol>) {
    if symbols.is_empty() {
        return;
    }
    extras.push((
        Expr::Symbol(Symbol::new(name)),
        Expr::List(symbols.into_iter().map(Expr::Symbol).collect()),
    ));
}

fn map_entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
