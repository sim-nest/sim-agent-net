//! Provider option-map parsing for HTTP model runners.

use crate::provider::ProviderProfile;
use sim_kernel::{CapabilityName, Cx, Error, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_net_core::{UrlParts, parse_url};
use sim_lib_provider::Secret;
use std::{collections::HashMap, time::Duration};

/// Concrete HTTP runner configuration derived from a provider profile and an
/// option map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Source provider profile.
    pub profile: ProviderProfile,
    /// Runner symbol after applying a `name` override.
    pub runner: Symbol,
    /// Codec symbol after applying a `codec` override.
    pub codec: Symbol,
    /// Base endpoint used by the HTTP runner.
    pub endpoint: String,
    /// Model name to send to the provider codec.
    pub model: String,
    /// Historical environment variable used by compatibility constructors.
    pub api_key_env: Option<String>,
    /// Credential resolved once while opening or constructing this seat.
    pub secret: Option<Secret>,
    /// Runner locality after endpoint posture has been classified.
    pub locality: Symbol,
    /// HTTP timeout.
    pub timeout: Duration,
    /// Whether requests should use streaming.
    pub stream: bool,
    /// Whether request encoding should include tools.
    pub tools: bool,
    /// Maximum decoded provider response size.
    pub max_output_bytes: usize,
    /// Grammar dialects this provider can enforce directly.
    pub grammar_dialects: Vec<sim_shape::GrammarDialect>,
}

impl ProviderConfig {
    /// Builds an opened seat configuration from already-admitted endpoint data.
    ///
    /// This path does not consult the environment or contact the endpoint.
    pub fn for_seat(
        profile: ProviderProfile,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        secret: Option<Secret>,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        let endpoint_parts = endpoint_parts(&endpoint)?;
        let locality = locality_for_endpoint(&profile, &endpoint_parts);
        Ok(Self {
            runner: profile.runner_symbol.clone(),
            codec: profile.codec.clone(),
            endpoint,
            model: model.into(),
            api_key_env: None,
            secret,
            locality,
            timeout: profile.default_timeout,
            stream: profile.default_stream,
            tools: profile.default_tools,
            max_output_bytes: profile.default_max_output_bytes,
            grammar_dialects: profile.grammar_dialects.clone(),
            profile,
        })
    }

    /// Builds provider config from the same option map used by the existing
    /// agent runner constructors.
    pub fn from_options(
        profile: ProviderProfile,
        cx: &mut Cx,
        options: &HashMap<String, Value>,
    ) -> Result<Self> {
        let runner = symbol_option(cx, options, "name", profile.runner_symbol.clone())?;
        let codec = symbol_option(cx, options, "codec", profile.codec.clone())?;
        let endpoint = endpoint_option(cx, options, &profile)?;
        let endpoint_parts = endpoint_parts(&endpoint)?;
        let model = stringish_option(cx, options, "model", &profile.default_model)?;
        let api_key_env = api_key_env_option(cx, options, &profile)?;
        let timeout = duration_option(cx, options, "timeout", profile.default_timeout)?;
        let stream = bool_option(cx, options, "stream", profile.default_stream)?;
        let tools = bool_option(cx, options, "tools", profile.default_tools)?;
        let max_output_bytes = usize_option(
            cx,
            options,
            "max-output-bytes",
            profile.default_max_output_bytes,
        )?;
        let locality = locality_for_endpoint(&profile, &endpoint_parts);
        let grammar_dialects = profile.grammar_dialects.clone();
        Ok(Self {
            profile,
            runner,
            codec,
            endpoint,
            model,
            api_key_env,
            secret: None,
            locality,
            timeout,
            stream,
            tools,
            max_output_bytes,
            grammar_dialects,
        })
    }

    /// Resolves the historical environment-backed credential once while the
    /// compatibility seat is being opened.
    pub fn resolve_compatibility_credential(&mut self, cx: &Cx) -> Result<()> {
        self.secret = resolve_compatibility_secret(cx, &self.profile, self.api_key_env.as_deref())?;
        Ok(())
    }

    /// Reports whether this opened config carries credential material.
    pub fn has_credential(&self) -> bool {
        self.secret.is_some()
    }
}

fn resolve_compatibility_secret(
    cx: &Cx,
    profile: &ProviderProfile,
    api_key_env: Option<&str>,
) -> Result<Option<Secret>> {
    let Some(env) = api_key_env else {
        return Ok(None);
    };
    cx.require(&CapabilityName::new("ai-runner-secret"))?;
    match std::env::var(env) {
        Ok(material) => Secret::new(material).map(Some),
        Err(_) if matches!(profile.auth, crate::ProviderAuth::OptionalBearerEnv { .. }) => Ok(None),
        Err(_) => Err(Error::Eval(format!(
            "provider seat credential is unavailable from compatibility source {env}"
        ))),
    }
}

pub(crate) fn compatibility_secret(env: &str) -> Result<Secret> {
    std::env::var(env)
        .map_err(|_| {
            Error::Eval(format!(
                "provider seat credential is unavailable from compatibility source {env}"
            ))
        })
        .and_then(Secret::new)
}

fn endpoint_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    profile: &ProviderProfile,
) -> Result<String> {
    let endpoint = stringish_option(cx, options, "endpoint", &profile.default_endpoint)?;
    if endpoint.is_empty() {
        return Err(Error::Eval(format!(
            "{} provider config requires endpoint",
            profile.provider
        )));
    }
    Ok(endpoint)
}

fn endpoint_parts(endpoint: &str) -> Result<UrlParts> {
    parse_url(endpoint).map_err(|err| Error::Eval(format!("invalid provider endpoint: {err}")))
}

fn locality_for_endpoint(profile: &ProviderProfile, endpoint: &UrlParts) -> Symbol {
    if profile.default_locality == Symbol::new("local") && !host_is_loopback(&endpoint.host) {
        Symbol::new("network")
    } else {
        profile.default_locality.clone()
    }
}

fn host_is_loopback(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn api_key_env_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    profile: &ProviderProfile,
) -> Result<Option<String>> {
    let Some(value) = options.get("api-key-env") else {
        return Ok(profile.auth.default_env().map(str::to_owned));
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(None),
        Expr::String(text) if text.is_empty() => Ok(None),
        Expr::String(text) => Ok(Some(text)),
        Expr::Symbol(symbol) => Ok(Some(symbol.to_string())),
        _ => Err(Error::Eval(
            "provider config :api-key-env expects nil, string, or symbol".to_owned(),
        )),
    }
}

fn stringish_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: &str,
) -> Result<String> {
    match options.get(key) {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            Expr::Symbol(symbol) => Ok(symbol.to_string()),
            _ => Err(Error::Eval(format!(
                "provider config :{key} expects a string or symbol"
            ))),
        },
        None => Ok(default.to_owned()),
    }
}

fn symbol_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: Symbol,
) -> Result<Symbol> {
    match options.get(key) {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::Symbol(symbol) => Ok(symbol),
            _ => Err(Error::Eval(format!(
                "provider config :{key} expects a symbol"
            ))),
        },
        None => Ok(default),
    }
}

fn bool_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool> {
    match options.get(key) {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::Bool(flag) => Ok(flag),
            _ => Err(Error::Eval(format!(
                "provider config :{key} expects a boolean"
            ))),
        },
        None => Ok(default),
    }
}

fn duration_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: Duration,
) -> Result<Duration> {
    match options.get(key) {
        Some(value) => parse_duration(&value.object().as_expr(cx)?),
        None => Ok(default),
    }
}

fn parse_duration(expr: &Expr) -> Result<Duration> {
    match expr {
        Expr::String(text) => parse_duration_text(text),
        Expr::Number(NumberLiteral { canonical, .. }) => canonical
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| Error::Eval("provider timeout expects integer milliseconds".to_owned())),
        _ => Err(Error::Eval(
            "provider timeout expects a duration string or integer milliseconds".to_owned(),
        )),
    }
}

fn parse_duration_text(text: &str) -> Result<Duration> {
    let (number, unit) = if let Some(number) = text.strip_suffix("ms") {
        (number, "ms")
    } else if let Some(number) = text.strip_suffix('s') {
        (number, "s")
    } else if let Some(number) = text.strip_suffix('m') {
        (number, "m")
    } else if let Some(number) = text.strip_suffix('h') {
        (number, "h")
    } else {
        return Err(Error::Eval(format!(
            "provider timeout {text} must end with ms, s, m, or h"
        )));
    };
    let value = number.parse::<u64>().map_err(|_| {
        Error::Eval(format!(
            "provider timeout {text} has an invalid numeric prefix"
        ))
    })?;
    Ok(match unit {
        "ms" => Duration::from_millis(value),
        "s" => Duration::from_secs(value),
        "m" => Duration::from_secs(value.saturating_mul(60)),
        "h" => Duration::from_secs(value.saturating_mul(60 * 60)),
        _ => unreachable!(),
    })
}

fn usize_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize> {
    match options.get(key) {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => parse_usize(&text, key),
            Expr::Number(NumberLiteral { canonical, .. }) => parse_usize(&canonical, key),
            _ => Err(Error::Eval(format!(
                "provider config :{key} expects an integer"
            ))),
        },
        None => Ok(default),
    }
}

fn parse_usize(text: &str, key: &str) -> Result<usize> {
    text.parse::<usize>()
        .map_err(|_| Error::Eval(format!("provider config :{key} expects an integer")))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
