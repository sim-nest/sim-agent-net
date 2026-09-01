use std::collections::BTreeSet;

use serde_json::Value;
use sim_codec_json::{
    JsonProjectionMode, ResourceIdentity, SchemaDocument, SchemaLimits, project_expr_to_json,
    project_json_to_expr,
};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};

pub(crate) fn default_parameters() -> Value {
    serde_json::json!({"type":"object","properties":{},"required":[]})
}

pub(crate) fn argument_order(parameters: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    if let Some(required) = parameters.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if seen.insert(name.to_owned()) {
                order.push(name.to_owned());
            }
        }
    }
    if let Some(properties) = parameters.get("properties").and_then(Value::as_object) {
        for name in properties.keys() {
            if seen.insert(name.clone()) {
                order.push(name.clone());
            }
        }
    }
    order
}

pub(crate) fn validate_schema(
    schema: &Value,
    value: &Value,
    path: &str,
) -> std::result::Result<(), String> {
    let document = compile_schema(schema, path)?;
    document.validate(value).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })
}

pub(crate) fn validate_supported_schema(
    schema: &Value,
    path: &str,
) -> std::result::Result<(), String> {
    compile_schema(schema, path).map(|_| ())
}

fn compile_schema(schema: &Value, source: &str) -> std::result::Result<SchemaDocument, String> {
    SchemaDocument::from_value(
        schema.clone(),
        ResourceIdentity {
            base_uri: format!("sim://openai-tool/{source}"),
            source: source.to_owned(),
        },
        SchemaLimits::default(),
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn json_to_expr(value: &Value) -> Expr {
    project_json_to_expr(value, JsonProjectionMode::UntaggedInterop)
}

pub(crate) fn json_to_value(cx: &mut Cx, value: &Value) -> Result<sim_kernel::Value> {
    match value {
        Value::Null => cx.factory().nil(),
        Value::Bool(value) => cx.factory().bool(*value),
        Value::Number(number) => cx
            .factory()
            .number_literal(Symbol::qualified("numbers", "f64"), number.to_string()),
        Value::String(value) => cx.factory().string(value.clone()),
        Value::Array(items) => items
            .iter()
            .map(|item| json_to_value(cx, item))
            .collect::<Result<Vec<_>>>()
            .and_then(|items| cx.factory().list(items)),
        Value::Object(entries) => entries
            .iter()
            .map(|(key, value)| {
                json_to_value(cx, value).map(|value| (Symbol::new(key.clone()), value))
            })
            .collect::<Result<Vec<_>>>()
            .and_then(|entries| cx.factory().table(entries)),
    }
}

pub(crate) fn arguments_json(expr: &Expr) -> Result<Value> {
    match expr {
        Expr::String(text) => serde_json::from_str(text)
            .map_err(|err| Error::Eval(format!("tool-call arguments must be JSON: {err}"))),
        other => Ok(project_expr_to_json(
            other,
            JsonProjectionMode::UntaggedInterop,
        )),
    }
}

pub(crate) fn expr_to_json_string(expr: &Expr) -> Value {
    project_expr_to_json(expr, JsonProjectionMode::TextLossy)
}

pub(crate) fn canonical_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

pub(crate) fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::String(text) => text.clone(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Bool(value) => value.to_string(),
        Expr::Nil => "nil".to_owned(),
        other => format!("{other:?}"),
    }
}
