use sim_kernel::{Error, Expr, Result, Symbol};

use crate::session::{DEFAULT_PROTOCOL_VERSION, McpSession};

pub(crate) fn initialize(session: &mut McpSession, params: Expr) -> Result<Expr> {
    match &params {
        Expr::Nil => {}
        Expr::Map(_) => {
            session.client_info = optional_field(&params, "clientInfo")
                .or_else(|| optional_field(&params, "client-info"))
                .cloned();
            if let Some(Expr::String(version)) = optional_field(&params, "protocolVersion")
                .or_else(|| optional_field(&params, "protocol-version"))
            {
                session.protocol_version = version.clone();
            }
        }
        _ => {
            return Err(Error::TypeMismatch {
                expected: "initialize params map or nil",
                found: "invalid initialize params",
            });
        }
    }

    Ok(Expr::Map(vec![
        field(
            "protocolVersion",
            Expr::String(session.protocol_version.clone()),
        ),
        field(
            "serverInfo",
            Expr::Map(vec![
                field("name", Expr::String("sim".to_owned())),
                field(
                    "version",
                    Expr::String(env!("CARGO_PKG_VERSION").to_owned()),
                ),
            ]),
        ),
        field(
            "capabilities",
            Expr::Map(vec![
                field("tools", Expr::Map(Vec::new())),
                field("resources", Expr::Map(Vec::new())),
                field("prompts", Expr::Map(Vec::new())),
            ]),
        ),
    ]))
}

pub(crate) fn initialized(session: &mut McpSession) -> Expr {
    session.initialized = true;
    empty_object()
}

pub(crate) fn ping() -> Expr {
    empty_object()
}

pub(crate) fn shutdown(session: &mut McpSession) -> Expr {
    session.shutdown_requested = true;
    empty_object()
}

pub(crate) fn health(session: &McpSession) -> Expr {
    Expr::Map(vec![
        field("kind", Expr::Symbol(Symbol::qualified("mcp", "health"))),
        field("session", Expr::String(session.id.clone())),
        field("initialized", Expr::Bool(session.initialized)),
        field("shutdown-requested", Expr::Bool(session.shutdown_requested)),
        field(
            "protocolVersion",
            Expr::String(session.protocol_version.clone()),
        ),
        field(
            "defaultProtocolVersion",
            Expr::String(DEFAULT_PROTOCOL_VERSION.to_owned()),
        ),
        field(
            "activeRequests",
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "i64"),
                canonical: session.active_requests.len().to_string(),
            }),
        ),
    ])
}

fn empty_object() -> Expr {
    Expr::Map(Vec::new())
}

fn optional_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(fields) = expr else {
        return None;
    };
    fields.iter().find_map(|(key, value)| {
        let field_name = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (field_name == name).then_some(value)
    })
}

use sim_value::build::entry as field;
