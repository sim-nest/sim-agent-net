use sim_citizen::value_from_expr;
use sim_kernel::{Cx, Error, Expr, Result, Value};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct McpCallParams {
    pub name: String,
    pub arguments: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct McpToolResult {
    pub content: Vec<Expr>,
    pub is_error: bool,
}

impl McpCallParams {
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP tools/call params")?;
        Ok(Self {
            name: required_string(fields, "name")?,
            arguments: optional_arguments(fields)?,
        })
    }
}

impl McpToolResult {
    pub fn success(cx: &mut Cx, value: Value) -> Result<Self> {
        Ok(Self {
            content: vec![value_to_content(cx, value)?],
            is_error: false,
        })
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![text_content(message.into())],
            is_error: true,
        }
    }

    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("content", Expr::List(self.content.clone())),
            field("isError", Expr::Bool(self.is_error)),
        ])
    }
}

pub(crate) fn arguments_to_values(cx: &mut Cx, arguments: &[Expr]) -> Result<Vec<Value>> {
    arguments
        .iter()
        .map(|expr| value_from_expr(cx, expr))
        .collect()
}

pub(crate) fn value_to_content(cx: &mut Cx, value: Value) -> Result<Expr> {
    let expr = value.object().as_expr(cx)?;
    Ok(match &expr {
        Expr::String(text) => text_content(text.clone()),
        Expr::Symbol(symbol) => text_content(symbol.to_string()),
        _ if is_json_safe(&expr) => json_content(expr),
        other => text_content(format!("{other:?}")),
    })
}

fn is_json_safe(expr: &Expr) -> bool {
    match expr {
        Expr::Nil | Expr::Bool(_) | Expr::Number(_) | Expr::String(_) => true,
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) => {
            items.iter().all(is_json_safe)
        }
        Expr::Map(fields) => fields
            .iter()
            .all(|(key, value)| json_key_safe(key) && is_json_safe(value)),
        _ => false,
    }
}

fn json_key_safe(expr: &Expr) -> bool {
    matches!(expr, Expr::String(_))
}

pub(crate) fn json_content(expr: Expr) -> Expr {
    Expr::Map(vec![
        field("type", Expr::String("json".to_owned())),
        field("json", expr),
    ])
}

pub(crate) fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        field("type", Expr::String("text".to_owned())),
        field("text", Expr::String(text)),
    ])
}

fn optional_arguments(fields: &[(Expr, Expr)]) -> Result<Vec<Expr>> {
    match optional_field(fields, "arguments") {
        Some(Expr::List(items)) => Ok(items.clone()),
        Some(Expr::Nil) | None => Ok(Vec::new()),
        Some(Expr::Map(fields)) => Ok(vec![Expr::Map(fields.clone())]),
        Some(_) => Err(Error::TypeMismatch {
            expected: "argument list, map, or nil",
            found: "invalid arguments",
        }),
    }
}

use sim_value::access::map_entries as map_fields;

fn required_string(fields: &[(Expr, Expr)], name: &str) -> Result<String> {
    match optional_field(fields, name) {
        Some(Expr::String(value)) => Ok(value.clone()),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
        None => Err(Error::TypeMismatch {
            expected: "required tools/call field",
            found: "missing field",
        }),
    }
}

fn optional_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    fields.iter().find_map(|(key, value)| {
        let key = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (key == name).then_some(value)
    })
}

pub(crate) use sim_value::build::entry as field;
