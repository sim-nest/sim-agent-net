use super::super::{
    model::{AgentComponent, ComponentBackend, RunnerBackend, component_value},
    options::{parse_component_options, path_option, string_option, symbol_option},
};
#[cfg(feature = "runner-http")]
use crate::AI_RUNNER_CAPABILITY;
#[cfg(feature = "runner-http")]
use crate::AI_RUNNER_NETWORK_CAPABILITY;
#[cfg(feature = "runner-http")]
use crate::AI_RUNNER_SECRET_CAPABILITY;
use crate::{ComponentKind, memory::load_memory_log, util::installed_codecs};
#[cfg(feature = "runner-http")]
use sim_kernel::CapabilityName;
use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};
#[cfg(feature = "runner-http")]
use sim_lib_agent_runner_http::HttpRunner;
#[cfg(feature = "runner-ollama")]
use sim_lib_agent_runner_http::{ProviderConfig, provider_profiles};
use sim_lib_server::{ServerAddress, parse_duration};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) fn runner_echo_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/echo")?;
    let symbol = symbol_option(cx, &options, "name", Symbol::qualified("runner", "echo"))?;
    let model = string_option(cx, &options, "model", "runner/echo")?;
    let mut spec = vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("echo"))),
        (Symbol::new("model"), Expr::String(model.clone())),
    ];
    spec.extend(runner_metadata_spec(cx, &options)?);
    component_value(
        cx,
        AgentComponent {
            symbol,
            kind: ComponentKind::Runner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Runner(RunnerBackend::Echo { model }),
        },
    )
}

pub(crate) fn runner_cassette_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/cassette")?;
    let symbol = symbol_option(
        cx,
        &options,
        "name",
        Symbol::qualified("runner", "cassette"),
    )?;
    let model = string_option(cx, &options, "model", "runner/cassette")?;
    let journal = path_option(cx, &options, "journal")?
        .ok_or_else(|| Error::Eval("runner/cassette :journal expects a string path".to_owned()))?;
    let strict = strict_option(cx, &options)?;
    let mut spec = vec![
        (
            Symbol::new("backend"),
            Expr::Symbol(Symbol::new("cassette")),
        ),
        (Symbol::new("model"), Expr::String(model.clone())),
        (
            Symbol::new("journal"),
            Expr::String(journal.display().to_string()),
        ),
        (Symbol::new("strict"), Expr::Bool(strict)),
    ];
    spec.extend(runner_metadata_spec(cx, &options)?);
    let entries = load_memory_log(&journal)?;
    component_value(
        cx,
        AgentComponent {
            symbol,
            kind: ComponentKind::Runner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Runner(RunnerBackend::Cassette {
                model,
                strict,
                entries: Arc::new(entries),
            }),
        },
    )
}

pub(crate) fn runner_fake_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/fake")?;
    let symbol = symbol_option(cx, &options, "name", Symbol::qualified("runner", "fake"))?;
    let model = string_option(cx, &options, "model", "runner/fake")?;
    let script = script_option(cx, &options)?;
    let delay = delay_option(cx, &options)?;
    let mut spec = vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("fake"))),
        (Symbol::new("model"), Expr::String(model.clone())),
        (Symbol::new("script"), Expr::List(script.clone())),
    ];
    if !delay.is_zero() {
        spec.push((
            Symbol::new("delay-ms"),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: delay.as_millis().to_string(),
            }),
        ));
    }
    spec.extend(runner_metadata_spec(cx, &options)?);
    component_value(
        cx,
        AgentComponent {
            symbol,
            kind: ComponentKind::Runner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Runner(RunnerBackend::Fake {
                model,
                script: Arc::new(Mutex::new(VecDeque::from(script))),
                delay,
            }),
        },
    )
}

#[cfg(feature = "runner-http")]
pub(crate) fn runner_openai_compatible_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/openai-compatible")?;
    let symbol = symbol_option(
        cx,
        &options,
        "name",
        Symbol::qualified("runner", "openai-compatible"),
    )?;
    let model = string_option(cx, &options, "model", "gpt-4.1-mini")?;
    let endpoint = string_option(cx, &options, "endpoint", "http://127.0.0.1:0/v1")?;
    let api_key_env = string_option(cx, &options, "api-key-env", "OPENAI_API_KEY")?;
    let codec = symbol_option(cx, &options, "codec", Symbol::qualified("codec", "openai"))?;
    let timeout = timeout_with_default(cx, &options, Duration::from_secs(60))?;
    let stream = bool_option(
        cx,
        &options,
        "stream",
        "runner/openai-compatible :stream expects a boolean",
        false,
    )?;
    let tools = bool_option(
        cx,
        &options,
        "tools",
        "runner/openai-compatible :tools expects a boolean",
        true,
    )?;
    let max_output_bytes = max_output_bytes_option(cx, &options, "runner/openai-compatible")?;
    component_value(
        cx,
        AgentComponent {
            symbol: symbol.clone(),
            kind: ComponentKind::Runner,
            capabilities: vec![
                CapabilityName::new(AI_RUNNER_CAPABILITY),
                CapabilityName::new(AI_RUNNER_NETWORK_CAPABILITY),
                CapabilityName::new(AI_RUNNER_SECRET_CAPABILITY),
            ],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("backend"),
                    Expr::Symbol(Symbol::new("openai-compatible")),
                ),
                (Symbol::new("model"), Expr::String(model.clone())),
                (Symbol::new("endpoint"), Expr::String(endpoint.clone())),
                (
                    Symbol::new("api-key-env"),
                    Expr::String(api_key_env.clone()),
                ),
                (Symbol::new("codec"), Expr::Symbol(codec.clone())),
                (
                    Symbol::new("timeout"),
                    Expr::String(format!("{}ms", timeout.as_millis())),
                ),
                (Symbol::new("stream"), Expr::Bool(stream)),
                (Symbol::new("tools"), Expr::Bool(tools)),
                (
                    Symbol::new("max-output-bytes"),
                    Expr::Number(sim_kernel::NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: max_output_bytes.to_string(),
                    }),
                ),
            ],
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: Arc::new(HttpRunner::new_openai_compatible(
                    symbol,
                    model,
                    endpoint,
                    api_key_env,
                    codec,
                    timeout,
                    stream,
                    tools,
                    max_output_bytes,
                )),
            }),
        },
    )
}

#[cfg(feature = "runner-ollama")]
pub(crate) fn runner_ollama_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = super::ai_provider_runner::provider_runner_options(cx, args, "runner/ollama")?;
    let config = ProviderConfig::from_options(provider_profiles::ollama(), cx, &options)?;
    super::ai_provider_runner::provider_runner_value(cx, config)
}

fn script_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Vec<Expr>> {
    let Some(value) = options.get("script") else {
        return Ok(Vec::new());
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => Ok(items),
        _ => Err(Error::Eval(
            "runner/fake :script expects a list or vector of Expr values".to_owned(),
        )),
    }
}

fn runner_metadata_spec(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Vec<(Symbol, Expr)>> {
    [
        "cost-usd",
        "context-tokens",
        "healthy",
        "health",
        "health-reason",
        "requires",
        "modalities",
        "modalities-in",
        "modalities-out",
        "privacy",
        "quality",
        "latency-ms",
        "delay-ms",
        "supports-stream",
        "supports-tools",
        "supports-json",
        "supports-shape",
    ]
    .into_iter()
    .filter_map(|key| {
        options.get(key).map(|value| {
            value
                .object()
                .as_expr(cx)
                .map(|expr| (Symbol::new(key), expr))
        })
    })
    .collect()
}

fn strict_option(cx: &mut Cx, options: &std::collections::HashMap<String, Value>) -> Result<bool> {
    let Some(value) = options.get("strict") else {
        return Ok(true);
    };
    match value.object().as_expr(cx)? {
        Expr::Bool(flag) => Ok(flag),
        _ => Err(Error::Eval(
            "runner/cassette :strict expects a boolean".to_owned(),
        )),
    }
}

fn delay_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Duration> {
    if let Some(value) = options.get("delay") {
        return parse_duration(&value.object().as_expr(cx)?);
    }
    match options.get("delay-ms") {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => text
                .parse::<u64>()
                .map(Duration::from_millis)
                .map_err(|_| Error::Eval("runner/fake :delay-ms expects an integer".to_owned())),
            Expr::Number(number) => number
                .canonical
                .parse::<u64>()
                .map(Duration::from_millis)
                .map_err(|_| Error::Eval("runner/fake :delay-ms expects an integer".to_owned())),
            _ => Err(Error::Eval(
                "runner/fake :delay-ms expects an integer".to_owned(),
            )),
        },
        None => Ok(Duration::ZERO),
    }
}

#[cfg(feature = "runner-http")]
fn timeout_with_default(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
    default: Duration,
) -> Result<Duration> {
    match options.get("timeout") {
        Some(value) => parse_duration(&value.object().as_expr(cx)?),
        None => Ok(default),
    }
}

#[cfg(feature = "runner-http")]
fn bool_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
    key: &str,
    error: &str,
    default: bool,
) -> Result<bool> {
    match options.get(key) {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::Bool(flag) => Ok(flag),
            _ => Err(Error::Eval(error.to_owned())),
        },
        None => Ok(default),
    }
}

#[cfg(feature = "runner-http")]
fn max_output_bytes_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
    runner_label: &str,
) -> Result<usize> {
    match options.get("max-output-bytes") {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => text.parse::<usize>().map_err(|_| {
                Error::Eval(format!(
                    "{runner_label} :max-output-bytes expects an integer"
                ))
            }),
            Expr::Number(number) => number.canonical.parse::<usize>().map_err(|_| {
                Error::Eval(format!(
                    "{runner_label} :max-output-bytes expects an integer"
                ))
            }),
            _ => Err(Error::Eval(format!(
                "{runner_label} :max-output-bytes expects an integer"
            ))),
        },
        None => Ok(1024 * 1024),
    }
}
