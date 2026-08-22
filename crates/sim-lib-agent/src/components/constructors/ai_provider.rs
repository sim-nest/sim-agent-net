use super::super::options::parse_component_options;
use super::ai_provider_runner::provider_runner_value;
use crate::{
    AI_RUNNER_CAPABILITY, AI_RUNNER_LOCAL_CAPABILITY, AI_RUNNER_NETWORK_CAPABILITY,
    AI_RUNNER_SECRET_CAPABILITY,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_agent_runner_http::{
    HttpProbeTransport, HttpRunner, ProviderAuth, ProviderConfig, ProviderProfile, probe_provider,
    provider_profiles,
};
use sim_lib_provider::{
    EndpointCard, PrincipalCard, ProviderAdapter, ProviderFamilyCard, ProviderRegistry,
    ProviderSeatCard, ProviderSeatId, ProviderSeatLimits,
};
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
    let config = ProviderConfig::from_options(registered_profile("openai")?, cx, &options)?;
    provider_runner_value(cx, config)
}

pub(crate) fn runner_anthropic_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/anthropic")?;
    let config = ProviderConfig::from_options(registered_profile("anthropic")?, cx, &options)?;
    provider_runner_value(cx, config)
}

pub(crate) fn runner_lm_studio_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/lm-studio")?;
    let config = ProviderConfig::from_options(registered_profile("lm-studio")?, cx, &options)?;
    provider_runner_value(cx, config)
}

pub(crate) fn runner_lemonade_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/lemonade")?;
    let config = ProviderConfig::from_options(registered_profile("lemonade")?, cx, &options)?;
    provider_runner_value(cx, config)
}

fn provider_profiles_expr() -> Expr {
    Expr::Map(
        registered_profiles()
            .expect("built-in provider profile registry must be valid")
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
        ProviderAuth::OptionalBearerEnv { env } => Expr::Map(vec![
            map_entry("kind", Expr::Symbol(Symbol::new("optional-bearer-env"))),
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
    registered_profiles().ok()?.into_iter().find(|profile| {
        name == profile.provider.to_string() || name == profile.runner_symbol.to_string()
    })
}

fn registered_profile(name: &str) -> Result<ProviderProfile> {
    provider_profile_named(name)
        .ok_or_else(|| Error::Eval(format!("unknown registered provider profile {name}")))
}

fn registered_profiles() -> Result<Vec<ProviderProfile>> {
    let mut registry = ProviderRegistry::new();
    let profiles = provider_profiles::all();
    for profile in &profiles {
        registry.register(Arc::new(HttpProfileRegistration(profile.clone())))?;
    }
    debug_assert_eq!(registry.families().len(), profiles.len());
    Ok(profiles)
}

struct HttpProfileRegistration(ProviderProfile);

impl ProviderAdapter for HttpProfileRegistration {
    fn family(&self) -> ProviderFamilyCard {
        ProviderFamilyCard {
            family: Symbol::qualified("provider", self.0.provider.name.as_ref()),
            transport: Symbol::new("http"),
            semantics: Symbol::new("model-turn"),
            auth_owner: Symbol::new("sim"),
            wires: vec![self.0.codec.clone()],
            operations: vec![Symbol::new("discover"), Symbol::new("open")],
            revision: provider_profile_expr(&self.0),
            extra: Vec::new(),
        }
    }

    fn discover(&self, _cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        let family = Symbol::qualified("provider", self.0.provider.name.as_ref());
        Ok(vec![ProviderSeatCard {
            seat: ProviderSeatId::new(family.clone(), "default")?,
            family,
            principal: PrincipalCard {
                label: "default".to_owned(),
                kind: provider_auth_kind(&self.0.auth),
                source: if matches!(self.0.auth, ProviderAuth::None) {
                    Symbol::new("none")
                } else {
                    Symbol::new("secret-provider")
                },
                digest: "redacted".to_owned(),
                extra: Vec::new(),
            },
            endpoint: EndpointCard {
                address: self.0.default_endpoint.clone(),
                transport: Symbol::new("http"),
                revision: Expr::Nil,
                extra: Vec::new(),
            },
            harness: None,
            model: Some(self.0.default_model.clone()),
            limits: ProviderSeatLimits::default(),
            revision: provider_profile_expr(&self.0),
            extra: Vec::new(),
        }])
    }

    fn open(
        &self,
        cx: &mut Cx,
        _seat: &ProviderSeatCard,
        options: Expr,
    ) -> Result<Arc<dyn sim_lib_provider::ModelRunner>> {
        if options != Expr::Nil {
            return Err(Error::Eval(
                "provider/open HTTP registration currently accepts nil options".to_owned(),
            ));
        }
        let mut config = ProviderConfig::from_options(self.0.clone(), cx, &HashMap::new())?;
        config.resolve_compatibility_credential(cx)?;
        Ok(Arc::new(HttpRunner::new_provider(config)))
    }
}

fn provider_auth_kind(auth: &ProviderAuth) -> Symbol {
    match auth {
        ProviderAuth::None => Symbol::new("none"),
        ProviderAuth::BearerEnv { .. } | ProviderAuth::OptionalBearerEnv { .. } => {
            Symbol::new("bearer")
        }
        ProviderAuth::HeaderEnv { .. } => Symbol::new("header"),
    }
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

fn map_entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}
