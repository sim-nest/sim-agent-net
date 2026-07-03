use super::super::{
    model::{AgentComponent, ComponentBackend, RetrieverBackend, SandboxBackend, component_value},
    options::{
        capabilities_option, expr_option, parse_component_options, path_option, string_option,
        strings_option,
    },
};
use crate::{
    ComponentKind, FILE_READ_CAPABILITY, NETWORK_CAPABILITY, SANDBOX_CAPABILITY,
    SANDBOX_SUBPROCESS_CAPABILITY, SANDBOX_WASM_CAPABILITY, util::installed_codecs,
};
use sim_kernel::{Args, CapabilityName, Cx, Expr, Result, Symbol, Value};
use sim_lib_server::{ServerAddress, parse_duration};
use std::path::PathBuf;

pub(crate) fn retriever_vector_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "retriever/vector")?;
    let store = string_option(cx, &options, "store", "default")?;
    let corpus = strings_option(cx, &options, "corpus")?;
    let mut spec = vec![(Symbol::new("store"), Expr::String(store.clone()))];
    if let Some(corpus_expr) = expr_option(cx, &options, "corpus")? {
        spec.push((Symbol::new("corpus"), corpus_expr));
    }
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("retriever", "vector"),
            kind: ComponentKind::Retriever,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Retriever(RetrieverBackend::Vector { store, corpus }),
        },
    )
}

pub(crate) fn retriever_web_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "retriever/web")?;
    let endpoint = string_option(cx, &options, "endpoint", "https://example.invalid")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("retriever", "web"),
            kind: ComponentKind::Retriever,
            capabilities: vec![CapabilityName::new(NETWORK_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(Symbol::new("endpoint"), Expr::String(endpoint.clone()))],
            backend: ComponentBackend::Retriever(RetrieverBackend::Web { endpoint }),
        },
    )
}

pub(crate) fn retriever_file_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "retriever/file")?;
    let root = path_option(cx, &options, "root")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("retriever", "file"),
            kind: ComponentKind::Retriever,
            capabilities: vec![CapabilityName::new(FILE_READ_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(
                Symbol::new("root"),
                root.as_ref()
                    .map(|path| Expr::String(path.display().to_string()))
                    .unwrap_or(Expr::Nil),
            )],
            backend: ComponentBackend::Retriever(RetrieverBackend::File { root }),
        },
    )
}

pub(crate) fn retriever_db_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "retriever/db")?;
    let path = match path_option(cx, &options, "path")? {
        Some(path) => path,
        None => PathBuf::from(string_option(cx, &options, "dsn", "memory://db")?),
    };
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("retriever", "db"),
            kind: ComponentKind::Retriever,
            capabilities: vec![CapabilityName::new(FILE_READ_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(
                Symbol::new("path"),
                Expr::String(path.display().to_string()),
            )],
            backend: ComponentBackend::Retriever(RetrieverBackend::Db { path }),
        },
    )
}

pub(crate) fn sandbox_wasm_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "sandbox/wasm")?;
    let region = string_option(cx, &options, "region", "default")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("sandbox", "wasm"),
            kind: ComponentKind::Sandbox,
            capabilities: vec![CapabilityName::new(SANDBOX_WASM_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(Symbol::new("region"), Expr::String(region.clone()))],
            backend: ComponentBackend::Sandbox(SandboxBackend::Wasm { region }),
        },
    )
}

pub(crate) fn sandbox_subprocess_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "sandbox/subprocess")?;
    let command = options
        .get("command")
        .map(|value| {
            crate::util::stringish_from_value(
                cx,
                value.clone(),
                "sandbox/subprocess :command expects a string or symbol",
            )
        })
        .transpose()?;
    let max_time = match options.get("max-time") {
        Some(value) => parse_duration(&value.object().as_expr(cx)?)?,
        None => std::time::Duration::from_secs(2),
    };
    let max_output_bytes = match options.get("max-output") {
        Some(value) => match value.object().as_expr(cx)? {
            Expr::String(text) => text.parse::<usize>().map_err(|_| {
                sim_kernel::Error::Eval(
                    "sandbox/subprocess :max-output expects an integer".to_owned(),
                )
            })?,
            Expr::Number(number) => number.canonical.parse::<usize>().map_err(|_| {
                sim_kernel::Error::Eval(
                    "sandbox/subprocess :max-output expects an integer".to_owned(),
                )
            })?,
            _ => {
                return Err(sim_kernel::Error::Eval(
                    "sandbox/subprocess :max-output expects an integer".to_owned(),
                ));
            }
        },
        None => 16 * 1024,
    };
    let mut spec = Vec::new();
    if let Some(command) = &command {
        spec.push((Symbol::new("command"), Expr::String(command.clone())));
    }
    spec.push((
        Symbol::new("max-time"),
        Expr::String(format!("{}ms", max_time.as_millis())),
    ));
    spec.push((
        Symbol::new("max-output"),
        Expr::Number(sim_kernel::NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: max_output_bytes.to_string(),
        }),
    ));
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("sandbox", "subprocess"),
            kind: ComponentKind::Sandbox,
            capabilities: vec![CapabilityName::new(SANDBOX_SUBPROCESS_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Sandbox(SandboxBackend::Subprocess {
                command,
                max_time,
                max_output_bytes,
            }),
        },
    )
}

pub(crate) fn sandbox_capability_restricted_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "sandbox/capability-restricted")?;
    let allowed = if options.contains_key("allow") {
        capabilities_option(cx, &options, "allow")?
    } else {
        capabilities_option(cx, &options, "allowed")?
    };
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("sandbox", "capability-restricted"),
            kind: ComponentKind::Sandbox,
            capabilities: vec![CapabilityName::new(SANDBOX_CAPABILITY)],
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(
                Symbol::new("allow"),
                Expr::List(
                    allowed
                        .iter()
                        .map(|capability| Expr::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            )],
            backend: ComponentBackend::Sandbox(SandboxBackend::CapabilityRestricted { allowed }),
        },
    )
}
