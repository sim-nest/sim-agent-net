use serde_json::{Map as JsonMap, Value as JsonValue};
use sim_codec_json::{
    JsonProjectionMode, ResourceIdentity, SchemaDocument, SchemaLimits, project_json_to_expr,
};
use sim_kernel::{Cx, Error, Expr, Result, ShapeRef, Symbol};

/// Converts a SIM `shape` into a JSON-Schema [`Expr`] map for MCP clients.
///
/// A missing shape maps to the permissive empty schema; core scalar shapes map
/// to their JSON-Schema `type`, and other shapes are carried through an
/// `x-sim-shape` extension key.
pub fn shape_to_json_schema(cx: &mut Cx, shape: Option<&ShapeRef>) -> Result<Expr> {
    let value = if let Some(shape) = shape {
        let expr = shape.object().as_expr(cx)?;
        schema_from_shape_expr(&expr)
    } else {
        any_schema()
    };
    let document = SchemaDocument::from_value(
        value,
        ResourceIdentity {
            base_uri: "sim://mcp/shape-schema".to_owned(),
            source: "mcp shape projection".to_owned(),
        },
        SchemaLimits::default(),
    )
    .map_err(|error| Error::Eval(format!("invalid MCP shape JSON Schema: {error}")))?;
    Ok(project_json_to_expr(
        document.value(),
        JsonProjectionMode::UntaggedInterop,
    ))
}

fn schema_from_shape_expr(expr: &Expr) -> JsonValue {
    match expr {
        Expr::Symbol(symbol) if symbol == &Symbol::qualified("core", "Any") => any_schema(),
        Expr::Symbol(symbol) if symbol == &Symbol::qualified("core", "String") => {
            typed_schema("string")
        }
        Expr::Symbol(symbol) if symbol == &Symbol::qualified("core", "Number") => {
            typed_schema("number")
        }
        Expr::Symbol(symbol) if symbol == &Symbol::qualified("core", "Bool") => {
            typed_schema("boolean")
        }
        Expr::Symbol(symbol) if symbol == &Symbol::qualified("core", "Nil") => typed_schema("null"),
        Expr::Symbol(symbol) => schema_with_sim_shape(symbol.to_string()),
        _ => schema_with_sim_shape(format!("{expr:?}")),
    }
}

fn any_schema() -> JsonValue {
    JsonValue::Object(JsonMap::new())
}

fn typed_schema(kind: &str) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert("type".to_owned(), JsonValue::String(kind.to_owned()));
    JsonValue::Object(object)
}

fn schema_with_sim_shape(shape: String) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert("x-sim-shape".to_owned(), JsonValue::String(shape));
    JsonValue::Object(object)
}
