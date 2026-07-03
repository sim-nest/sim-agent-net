use sim_kernel::{Cx, Expr, Result, ShapeRef, Symbol};

/// Converts a SIM `shape` into a JSON-Schema [`Expr`] map for MCP clients.
///
/// A missing shape maps to the permissive empty schema; core scalar shapes map
/// to their JSON-Schema `type`, and other shapes are carried through an
/// `x-sim-shape` extension key.
pub fn shape_to_json_schema(cx: &mut Cx, shape: Option<&ShapeRef>) -> Result<Expr> {
    let Some(shape) = shape else {
        return Ok(any_schema());
    };
    let expr = shape.object().as_expr(cx)?;
    Ok(schema_from_shape_expr(&expr))
}

fn schema_from_shape_expr(expr: &Expr) -> Expr {
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

fn any_schema() -> Expr {
    Expr::Map(Vec::new())
}

fn typed_schema(kind: &str) -> Expr {
    Expr::Map(vec![field("type", Expr::String(kind.to_owned()))])
}

fn schema_with_sim_shape(shape: String) -> Expr {
    Expr::Map(vec![field("x-sim-shape", Expr::String(shape))])
}

use sim_value::build::entry as field;
