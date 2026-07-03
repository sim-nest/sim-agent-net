use sim_kernel::{Error, Expr};

use crate::content::field;

const NOT_FOUND_PREFIX: &str = "mcp.not-found|";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpResourceUriKind {
    Sim,
    Skill,
    Unsupported,
}

pub(crate) fn resource_uri_kind(uri: &str) -> McpResourceUriKind {
    if uri.starts_with("sim://") {
        McpResourceUriKind::Sim
    } else if uri.starts_with("skill://") {
        McpResourceUriKind::Skill
    } else {
        McpResourceUriKind::Unsupported
    }
}

pub(crate) fn not_found_error(kind: &str, id: impl AsRef<str>) -> Error {
    Error::Eval(format!("{NOT_FOUND_PREFIX}{kind}|{}", id.as_ref()))
}

pub(crate) fn not_found_error_data(error: &Error) -> Option<Expr> {
    let Error::Eval(message) = error else {
        return None;
    };
    let rest = message.strip_prefix(NOT_FOUND_PREFIX)?;
    let (kind, id) = rest.split_once('|')?;
    Some(Expr::Map(vec![
        field("code", Expr::String("not-found".to_owned())),
        field("kind", Expr::String(kind.to_owned())),
        field("id", Expr::String(id.to_owned())),
        field("message", Expr::String(format!("{kind} not found: {id}"))),
    ]))
}

pub(crate) fn optional_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    fields.iter().find_map(|(key, value)| {
        let key = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (key == name).then_some(value)
    })
}

pub(crate) fn required_string_field(fields: &[(Expr, Expr)], name: &str) -> Result<String, Error> {
    match optional_field(fields, name) {
        Some(Expr::String(value)) => Ok(value.clone()),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
        None => Err(Error::TypeMismatch {
            expected: "required MCP field",
            found: "missing field",
        }),
    }
}
