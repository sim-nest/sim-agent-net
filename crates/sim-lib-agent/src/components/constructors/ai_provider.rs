use super::super::options::parse_component_options;
use crate::{
    AI_RUNNER_CAPABILITY, AI_RUNNER_LOCAL_CAPABILITY, AI_RUNNER_NETWORK_CAPABILITY,
    AI_RUNNER_SECRET_CAPABILITY,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_agent_runner_http::{
    HttpProbeTransport, ProviderAuth, ProviderConfig, ProviderProfile, probe_provider,
    provider_profiles,
};
use std::collections::HashMap;

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
        expr_field("provider", Expr::Symbol(profile.provider.clone())),
        expr_field("runner", Expr::Symbol(profile.runner_symbol.clone())),
        expr_field("codec", Expr::Symbol(profile.codec.clone())),
        expr_field("endpoint", Expr::String(profile.default_endpoint.clone())),
        expr_field("models-path", Expr::String(profile.models_path.to_owned())),
        expr_field("chat-path", Expr::String(profile.chat_path.to_owned())),
        expr_field("auth", provider_auth_expr(&profile.auth)),
        expr_field("locality", Expr::Symbol(profile.default_locality.clone())),
        expr_field("model", Expr::String(profile.default_model.clone())),
        expr_field(
            "timeout-ms",
            number_expr(profile.default_timeout.as_millis()),
        ),
        expr_field("stream", Expr::Bool(profile.default_stream)),
        expr_field("tools", Expr::Bool(profile.default_tools)),
        expr_field(
            "max-output-bytes",
            number_expr(profile.default_max_output_bytes),
        ),
    ])
}

fn provider_auth_expr(auth: &ProviderAuth) -> Expr {
    match auth {
        ProviderAuth::None => {
            Expr::Map(vec![expr_field("kind", Expr::Symbol(Symbol::new("none")))])
        }
        ProviderAuth::BearerEnv { env } => Expr::Map(vec![
            expr_field("kind", Expr::Symbol(Symbol::new("bearer-env"))),
            expr_field("env", Expr::String(env.clone())),
        ]),
        ProviderAuth::HeaderEnv { header, env } => Expr::Map(vec![
            expr_field("kind", Expr::Symbol(Symbol::new("header-env"))),
            expr_field("header", Expr::String(header.clone())),
            expr_field("env", Expr::String(env.clone())),
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

fn expr_field(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}
