use super::super::{
    model::{AgentComponent, ComponentBackend, RunnerBackend, component_value},
    options::parse_component_options,
};
use crate::{
    AI_RUNNER_CAPABILITY, AI_RUNNER_LOCAL_CAPABILITY, AI_RUNNER_NETWORK_CAPABILITY,
    AI_RUNNER_SECRET_CAPABILITY, ComponentKind, util::installed_codecs,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_agent_runner_http::{
    HttpProbeTransport, HttpRunner, ProviderAuth, ProviderConfig, ProviderProfile, probe_provider,
    provider_profiles,
};
use sim_lib_server::ServerAddress;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn provider_profiles_value(cx: &mut Cx, args: Args) -> Result<Value> {
    if !args.values().is_empty() {
        return Err(Error::Eval(
            "provider/profiles does not accept arguments".to_owned(),
        ));
    }
    crate::value_from_expr(cx, &provider_profiles_expr())
}

pub(crate) fn provider_probe_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "provider/probe")?;
    let profile = provider_profile_option(cx, &options)?;
    let config = ProviderConfig::from_options(profile, cx, &options)?;
    require_provider_probe_capabilities(cx, &config)?;
    let report = probe_provider(&HttpProbeTransport, &config)?;
    crate::value_from_expr(cx, &report.to_expr())
}

pub(crate) fn runner_openai_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/openai")?;
    let config = ProviderConfig::from_options(provider_profiles::openai(), cx, &options)?;
    provider_runner_value(cx, config)
}

pub(crate) fn runner_anthropic_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/anthropic")?;
    let config = ProviderConfig::from_options(provider_profiles::anthropic(), cx, &options)?;
    provider_runner_value(cx, config)
}

fn provider_profiles_expr() -> Expr {
    Expr::Map(
        provider_profiles::all()
            .into_iter()
            .map(|profile| {
                (
                    Expr::Symbol(profile.provider.clone()),
                    provider_profile_expr(&profile),
                )
            })
            .collect(),
    )
}

fn provider_profile_expr(profile: &ProviderProfile) -> Expr {
    Expr::Map(vec![
        map_entry("provider", Expr::Symbol(profile.provider.clone())),
        map_entry("runner", Expr::Symbol(profile.runner_symbol.clone())),
        map_entry("codec", Expr::Symbol(profile.codec.clone())),
        map_entry("endpoint", Expr::String(profile.default_endpoint.clone())),
        map_entry("models-path", Expr::String(profile.models_path.to_owned())),
        map_entry("chat-path", Expr::String(profile.chat_path.to_owned())),
        map_entry("auth", provider_auth_expr(&profile.auth)),
        map_entry("locality", Expr::Symbol(profile.default_locality.clone())),
        map_entry("model", Expr::String(profile.default_model.clone())),
        map_entry(
            "timeout-ms",
            number_expr(profile.default_timeout.as_millis()),
        ),
        map_entry("stream", Expr::Bool(profile.default_stream)),
        map_entry("tools", Expr::Bool(profile.default_tools)),
        map_entry(
            "max-output-bytes",
            number_expr(profile.default_max_output_bytes),
        ),
    ])
}

fn provider_auth_expr(auth: &ProviderAuth) -> Expr {
    match auth {
        ProviderAuth::None => Expr::Map(vec![map_entry("kind", Expr::Symbol(Symbol::new("none")))]),
        ProviderAuth::BearerEnv { env } => Expr::Map(vec![
            map_entry("kind", Expr::Symbol(Symbol::new("bearer-env"))),
            map_entry("env", Expr::String(env.clone())),
        ]),
        ProviderAuth::HeaderEnv { header, env } => Expr::Map(vec![
            map_entry("kind", Expr::Symbol(Symbol::new("header-env"))),
            map_entry("header", Expr::String(header.clone())),
            map_entry("env", Expr::String(env.clone())),
        ]),
    }
}

fn provider_profile_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
) -> Result<ProviderProfile> {
    let Some(value) = options.get("provider").or_else(|| options.get("runner")) else {
        return Ok(provider_profiles::openai_compatible());
    };
    let name = match value.object().as_expr(cx)? {
        Expr::String(text) => text,
        Expr::Symbol(symbol) => symbol.to_string(),
        _ => {
            return Err(Error::Eval(
                "provider/probe :provider expects a string or symbol".to_owned(),
            ));
        }
    };
    provider_profile_named(&name)
        .ok_or_else(|| Error::Eval(format!("unknown provider profile {name}")))
}

fn provider_profile_named(name: &str) -> Option<ProviderProfile> {
    provider_profiles::all().into_iter().find(|profile| {
        name == profile.provider.to_string() || name == profile.runner_symbol.to_string()
    })
}

fn require_provider_probe_capabilities(cx: &Cx, config: &ProviderConfig) -> Result<()> {
    cx.require(&CapabilityName::new(AI_RUNNER_CAPABILITY))?;
    if config.locality == Symbol::new("local") {
        cx.require(&CapabilityName::new(AI_RUNNER_LOCAL_CAPABILITY))?;
    } else {
        cx.require(&CapabilityName::new(AI_RUNNER_NETWORK_CAPABILITY))?;
    }
    if config.api_key_env.is_some() {
        cx.require(&CapabilityName::new(AI_RUNNER_SECRET_CAPABILITY))?;
    }
    Ok(())
}

fn provider_runner_value(cx: &mut Cx, config: ProviderConfig) -> Result<Value> {
    let symbol = config.runner.clone();
    let capabilities = provider_runner_capabilities(&config);
    let spec = provider_runner_spec(&config);
    component_value(
        cx,
        AgentComponent {
            symbol,
            kind: ComponentKind::Runner,
            capabilities,
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: Arc::new(HttpRunner::new_provider(config)),
            }),
        },
    )
}

fn provider_runner_capabilities(config: &ProviderConfig) -> Vec<CapabilityName> {
    let mut capabilities = vec![CapabilityName::new(AI_RUNNER_CAPABILITY)];
    if config.locality == Symbol::new("local") {
        capabilities.push(CapabilityName::new(AI_RUNNER_LOCAL_CAPABILITY));
    } else {
        capabilities.push(CapabilityName::new(AI_RUNNER_NETWORK_CAPABILITY));
    }
    if config.api_key_env.is_some() {
        capabilities.push(CapabilityName::new(AI_RUNNER_SECRET_CAPABILITY));
    }
    capabilities
}

fn provider_runner_spec(config: &ProviderConfig) -> Vec<(Symbol, Expr)> {
    let mut spec = vec![
        (
            Symbol::new("backend"),
            Expr::Symbol(config.profile.provider.clone()),
        ),
        (
            Symbol::new("provider"),
            Expr::Symbol(config.profile.provider.clone()),
        ),
        (
            Symbol::new("locality"),
            Expr::Symbol(config.locality.clone()),
        ),
        (Symbol::new("model"), Expr::String(config.model.clone())),
        (
            Symbol::new("endpoint"),
            Expr::String(config.endpoint.clone()),
        ),
        (Symbol::new("codec"), Expr::Symbol(config.codec.clone())),
        (
            Symbol::new("timeout"),
            Expr::String(format!("{}ms", config.timeout.as_millis())),
        ),
        (Symbol::new("stream"), Expr::Bool(config.stream)),
        (Symbol::new("tools"), Expr::Bool(config.tools)),
        (
            Symbol::new("max-output-bytes"),
            Expr::Number(NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: config.max_output_bytes.to_string(),
            }),
        ),
    ];
    if let Some(api_key_env) = &config.api_key_env {
        spec.push((
            Symbol::new("api-key-env"),
            Expr::String(api_key_env.clone()),
        ));
    }
    spec
}

fn map_entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}
