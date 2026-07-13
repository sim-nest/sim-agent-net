//! Provider health probing for HTTP model profiles.

mod transport;

use crate::{ProviderAuth, ProviderConfig, redact::redact_text};
use sim_kernel::{Error, Expr, Result, Symbol};
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProbeReport {
    /// Provider profile id.
    pub provider: Symbol,
    /// Endpoint that was probed.
    pub endpoint: String,
    /// Probe outcome.
    pub status: ProbeStatus,
    /// Models discovered from the provider model-list response.
    pub models: Vec<String>,
    /// Redacted failure or skip reason.
    pub reason: Option<String>,
    /// Whether the profile has an authentication shape whose secret is redacted.
    pub redacted: bool,
}

impl ProviderProbeReport {
    /// Converts this report to table-visible expression data.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            symbol_field("provider", Expr::Symbol(self.provider.clone())),
            symbol_field("endpoint", Expr::String(self.endpoint.clone())),
            symbol_field("status", Expr::Symbol(self.status.as_symbol())),
            symbol_field(
                "models",
                Expr::List(self.models.iter().cloned().map(Expr::String).collect()),
            ),
            symbol_field(
                "reason",
                self.reason
                    .as_ref()
                    .map(|reason| Expr::String(reason.clone()))
                    .unwrap_or(Expr::Nil),
            ),
            symbol_field("redacted", Expr::Bool(self.redacted)),
        ])
    }
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
    let redacted = config.profile.auth != ProviderAuth::None;
    let mut report = ProviderProbeReport {
        provider: config.profile.provider.clone(),
        endpoint: config.endpoint.clone(),
        status: ProbeStatus::Unavailable,
        models: Vec::new(),
        reason: None,
        redacted,
    };
    let endpoint_parts = parse_url(&config.endpoint)
        .map_err(|error| Error::Eval(format!("invalid provider endpoint: {error}")))?;

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
            report.status = ProbeStatus::Skipped;
            report.reason = Some(format!("missing secret env {env}"));
            return Ok(report);
        }
        AuthHeader::NoHeader => {}
    }
    if config.profile.provider == Symbol::new("anthropic") {
        headers.push(("anthropic-version".to_owned(), "2023-06-01".to_owned()));
    }

    let request = ProbeHttpRequest {
        endpoint: &config.endpoint,
        endpoint_parts,
        path: config.profile.models_path,
        headers,
        timeout: config.timeout,
        max_response_bytes: config.max_output_bytes,
    };
    let response = match transport.get(request) {
        Ok(response) => response,
        Err(error) => {
            report.reason = Some(redact_error(error, &secrets));
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
    match parse_provider_models(&config.profile.provider, &response.body) {
        Ok(models) => {
            report.status = ProbeStatus::Available;
            report.models = models;
        }
        Err(reason) => {
            report.reason = Some(reason);
        }
    }
    Ok(report)
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
        (ProviderAuth::BearerEnv { .. }, Some(env)) => match secret_from_env(env) {
            Some(secret) => Ok(AuthHeader::Header {
                name: "Authorization".to_owned(),
                value: format!("Bearer {secret}"),
                secret,
            }),
            None => Ok(AuthHeader::MissingSecret {
                env: env.to_owned(),
            }),
        },
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

fn secret_from_env(env: &str) -> Option<String> {
    match std::env::var(env) {
        Ok(secret) if !secret.is_empty() => Some(secret),
        _ => None,
    }
}

fn parse_provider_models(
    provider: &Symbol,
    body: &[u8],
) -> std::result::Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| format!("malformed model list json: {error}"))?;
    if provider == &Symbol::new("ollama") {
        parse_ollama_models(&value)
    } else {
        parse_openai_style_models(&value)
    }
}

fn parse_openai_style_models(
    value: &serde_json::Value,
) -> std::result::Result<Vec<String>, String> {
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "model list json missing data array".to_owned())?;
    Ok(data
        .iter()
        .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
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

fn symbol_field(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
