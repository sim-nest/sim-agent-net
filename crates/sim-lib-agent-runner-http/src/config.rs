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
mod tests {
    use super::ProviderConfig;
    use crate::provider_profiles;
    use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Expr, NumberLiteral, Symbol, Value};
    use std::{collections::HashMap, sync::Arc, time::Duration};

    #[test]
    fn parses_current_openai_compatible_option_map() {
        let mut cx = test_cx();
        let mut options = HashMap::new();
        insert(
            &mut cx,
            &mut options,
            "name",
            Expr::Symbol(Symbol::new("hosted")),
        );
        insert(
            &mut cx,
            &mut options,
            "model",
            Expr::String("model-a".to_owned()),
        );
        insert(
            &mut cx,
            &mut options,
            "endpoint",
            Expr::String("http://127.0.0.1:8080/v1".to_owned()),
        );
        insert(
            &mut cx,
            &mut options,
            "api-key-env",
            Expr::String("TEST_KEY".to_owned()),
        );
        insert(
            &mut cx,
            &mut options,
            "codec",
            Expr::Symbol(Symbol::qualified("codec", "openai")),
        );
        insert(
            &mut cx,
            &mut options,
            "timeout",
            Expr::String("750ms".to_owned()),
        );
        insert(&mut cx, &mut options, "stream", Expr::Bool(true));
        insert(&mut cx, &mut options, "tools", Expr::Bool(false));
        insert(&mut cx, &mut options, "max-output-bytes", number("4096"));

        let config =
            ProviderConfig::from_options(provider_profiles::openai_compatible(), &mut cx, &options)
                .unwrap();

        assert_eq!(config.runner, Symbol::new("hosted"));
        assert_eq!(config.codec, Symbol::qualified("codec", "openai"));
        assert_eq!(config.endpoint, "http://127.0.0.1:8080/v1");
        assert_eq!(config.model, "model-a");
        assert_eq!(config.api_key_env, Some("TEST_KEY".to_owned()));
        assert_eq!(config.locality, Symbol::new("network"));
        assert_eq!(config.timeout, Duration::from_millis(750));
        assert!(config.stream);
        assert!(!config.tools);
        assert_eq!(config.max_output_bytes, 4096);
    }

    #[test]
    fn generic_profile_requires_endpoint() {
        let mut cx = test_cx();
        let error = ProviderConfig::from_options(
            provider_profiles::openai_compatible(),
            &mut cx,
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires endpoint"));
    }

    #[test]
    fn ollama_defaults_preserve_existing_constructor_behavior() {
        let mut cx = test_cx();
        let config =
            ProviderConfig::from_options(provider_profiles::ollama(), &mut cx, &HashMap::new())
                .unwrap();

        assert_eq!(config.runner, Symbol::qualified("runner", "ollama"));
        assert_eq!(config.codec, Symbol::qualified("codec", "ollama"));
        assert_eq!(config.endpoint, "http://127.0.0.1:11434");
        assert_eq!(config.model, "qwen3.5:4b");
        assert_eq!(config.api_key_env, None);
        assert_eq!(config.locality, Symbol::new("local"));
        assert_eq!(config.timeout, Duration::from_secs(120));
        assert!(config.stream);
        assert!(!config.tools);
        assert_eq!(config.max_output_bytes, 1024 * 1024);
    }

    #[test]
    fn local_profile_becomes_network_when_endpoint_is_not_loopback() {
        let mut cx = test_cx();
        let mut options = HashMap::new();
        insert(
            &mut cx,
            &mut options,
            "endpoint",
            Expr::String("https://models.example/v1".to_owned()),
        );

        let config =
            ProviderConfig::from_options(provider_profiles::lm_studio(), &mut cx, &options)
                .unwrap();

        assert_eq!(config.locality, Symbol::new("network"));
        assert_eq!(config.api_key_env, None);
    }

    #[test]
    fn lm_studio_auth_env_is_optional_but_accepted() {
        let mut cx = test_cx();
        let mut options = HashMap::new();
        insert(
            &mut cx,
            &mut options,
            "api-key-env",
            Expr::String("LM_STUDIO_API_KEY".to_owned()),
        );

        let config =
            ProviderConfig::from_options(provider_profiles::lm_studio(), &mut cx, &options)
                .unwrap();

        assert_eq!(config.api_key_env, Some("LM_STUDIO_API_KEY".to_owned()));
        assert_eq!(config.locality, Symbol::new("local"));
    }

    #[test]
    fn nil_api_key_env_disables_profile_default_auth_env() {
        let mut cx = test_cx();
        let mut options = HashMap::new();
        insert(&mut cx, &mut options, "api-key-env", Expr::Nil);

        let config =
            ProviderConfig::from_options(provider_profiles::openai(), &mut cx, &options).unwrap();

        assert_eq!(config.api_key_env, None);
    }

    fn test_cx() -> Cx {
        Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
    }

    fn insert(cx: &mut Cx, options: &mut HashMap<String, Value>, key: &str, expr: Expr) {
        options.insert(key.to_owned(), cx.factory().expr(expr).unwrap());
    }

    fn number(canonical: &str) -> Expr {
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: canonical.to_owned(),
        })
    }
}
