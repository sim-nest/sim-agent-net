use super::super::model::{AgentComponent, ComponentBackend, RunnerBackend, component_value};
#[cfg(feature = "runner-ollama")]
use super::super::options::parse_component_options;
use crate::{
    AI_RUNNER_CAPABILITY, AI_RUNNER_LOCAL_CAPABILITY, AI_RUNNER_NETWORK_CAPABILITY,
    AI_RUNNER_SECRET_CAPABILITY, ComponentKind, util::installed_codecs,
};
use sim_kernel::{CapabilityName, Cx, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_agent_runner_http::{HttpRunner, ProviderConfig};
use sim_lib_server::ServerAddress;
use std::sync::Arc;

#[cfg(feature = "runner-ollama")]
pub(super) fn provider_runner_options(
    cx: &mut Cx,
    args: sim_kernel::Args,
    runner_label: &'static str,
) -> Result<std::collections::HashMap<String, Value>> {
    parse_component_options(cx, args, runner_label)
}

pub(super) fn provider_runner_value(cx: &mut Cx, mut config: ProviderConfig) -> Result<Value> {
    config.resolve_compatibility_credential(cx)?;
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
    if config.has_credential() {
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
