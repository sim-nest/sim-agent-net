use super::super::{
    model::{AgentComponent, ComponentBackend, RunnerBackend, component_value},
    options::{parse_component_options, string_option, symbol_option},
};
use crate::{
    AI_RUNNER_CAPABILITY, AI_RUNNER_LOCAL_CAPABILITY, ComponentKind, util::installed_codecs,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_agent_runner_process::{ProcessProtocol, ProcessRunner};
use sim_lib_server::{ServerAddress, parse_duration};
use std::{sync::Arc, time::Duration};

pub(crate) fn runner_process_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/process")?;
    let symbol = symbol_option(cx, &options, "name", Symbol::qualified("runner", "process"))?;
    let model = string_option(cx, &options, "model", "runner/process")?;
    let command = options
        .get("command")
        .ok_or_else(|| Error::Eval("runner/process :command expects a string or symbol".to_owned()))
        .and_then(|value| {
            crate::util::stringish_from_value(
                cx,
                value.clone(),
                "runner/process :command expects a string or symbol",
            )
        })?;
    let protocol = protocol_option(cx, &options)?;
    let timeout = timeout_option(cx, &options)?;
    let max_output_bytes = max_output_bytes_option(cx, &options)?;
    component_value(
        cx,
        AgentComponent {
            symbol: symbol.clone(),
            kind: ComponentKind::Runner,
            capabilities: vec![
                CapabilityName::new(AI_RUNNER_CAPABILITY),
                CapabilityName::new(AI_RUNNER_LOCAL_CAPABILITY),
            ],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: runner_spec(&model, &command, protocol, timeout, max_output_bytes),
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: Arc::new(ProcessRunner::new(
                    symbol,
                    model,
                    command,
                    protocol,
                    timeout,
                    max_output_bytes,
                )),
            }),
        },
    )
}

fn runner_spec(
    model: &str,
    command: &str,
    protocol: ProcessProtocol,
    timeout: Duration,
    max_output_bytes: usize,
) -> Vec<(Symbol, Expr)> {
    vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("process"))),
        (Symbol::new("model"), Expr::String(model.to_owned())),
        (Symbol::new("command"), Expr::String(command.to_owned())),
        (
            Symbol::new("protocol"),
            Expr::Symbol(Symbol::new(match protocol {
                ProcessProtocol::JsonStdio => "json-stdio",
                ProcessProtocol::LineText => "line-text",
            })),
        ),
        (
            Symbol::new("timeout"),
            Expr::String(format!("{}ms", timeout.as_millis())),
        ),
        (
            Symbol::new("max-output-bytes"),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: max_output_bytes.to_string(),
            }),
        ),
    ]
}

fn protocol_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<ProcessProtocol> {
    let protocol = options
        .get("protocol")
        .map(|value| {
            crate::util::stringish_from_value(
                cx,
                value.clone(),
                "runner/process :protocol expects a string or symbol",
            )
        })
        .transpose()?
        .unwrap_or_else(|| "json-stdio".to_owned());
    match protocol.as_str() {
        "json-stdio" => Ok(ProcessProtocol::JsonStdio),
        "line-text" => Ok(ProcessProtocol::LineText),
        _ => Err(Error::Eval(
            "runner/process :protocol expects json-stdio or line-text".to_owned(),
        )),
    }
}

fn timeout_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Duration> {
    match options.get("timeout") {
        Some(value) => parse_duration(&value.object().as_expr(cx)?),
        None => Ok(Duration::from_secs(30)),
    }
}

fn max_output_bytes_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<usize> {
    match options.get("max-output-bytes") {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => parse_max_output_bytes(&text),
            Expr::Number(number) => parse_max_output_bytes(&number.canonical),
            _ => Err(max_output_bytes_error()),
        },
        None => Ok(1024 * 1024),
    }
}

fn parse_max_output_bytes(text: &str) -> Result<usize> {
    text.parse::<usize>().map_err(|_| max_output_bytes_error())
}

fn max_output_bytes_error() -> Error {
    Error::Eval("runner/process :max-output-bytes expects an integer".to_owned())
}
