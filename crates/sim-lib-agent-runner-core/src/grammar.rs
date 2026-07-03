use sim_kernel::{Error, Expr, Result};
use sim_shape::{
    AnyShape, ExactExprShape, ExprKind, ExprKindShape, FieldShape, ListShape, OneOfShape, Shape,
};

/// Lowers a SIM `shape` into a constrained-decoding grammar string.
///
/// The grammar can be handed to a model runner to constrain generation to
/// values that match `shape`.
pub fn shape_to_grammar(shape: &dyn Shape) -> Result<String> {
    lower_shape(shape)
}

fn lower_shape(shape: &dyn Shape) -> Result<String> {
    if shape.as_any().is::<AnyShape>() {
        return Ok("true".to_owned());
    }
    if let Some(kind) = shape.as_any().downcast_ref::<ExprKindShape>() {
        return lower_expr_kind(kind.kind());
    }
    if let Some(fields) = shape.as_any().downcast_ref::<FieldShape>() {
        return lower_field_shape(fields);
    }
    if let Some(list) = shape.as_any().downcast_ref::<ListShape>() {
        return lower_list_shape(list);
    }
    if let Some(one_of) = shape.as_any().downcast_ref::<OneOfShape>() {
        let choices = one_of
            .choices()
            .iter()
            .map(|choice| lower_shape(choice.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        return Ok(format!(r#"{{"anyOf":[{}]}}"#, choices.join(",")));
    }
    if let Some(exact) = shape.as_any().downcast_ref::<ExactExprShape>() {
        return Ok(format!(r#"{{"const":{}}}"#, json_expr(exact.expected())?));
    }
    Err(Error::Eval(
        "shape_to_grammar does not support this shape".to_owned(),
    ))
}

fn lower_expr_kind(kind: &ExprKind) -> Result<String> {
    Ok(match kind {
        ExprKind::Nil => r#"{"type":"null"}"#.to_owned(),
        ExprKind::Bool => r#"{"type":"boolean"}"#.to_owned(),
        ExprKind::Number => r#"{"type":"number"}"#.to_owned(),
        ExprKind::String => r#"{"type":"string"}"#.to_owned(),
        ExprKind::List | ExprKind::Vector => r#"{"type":"array"}"#.to_owned(),
        ExprKind::Map => r#"{"type":"object"}"#.to_owned(),
        ExprKind::Symbol => r#"{"type":"string","description":"symbol"}"#.to_owned(),
        other => {
            return Err(Error::Eval(format!(
                "shape_to_grammar does not support expr-kind {}",
                other.name()
            )));
        }
    })
}

fn lower_field_shape(shape: &FieldShape) -> Result<String> {
    let properties = shape
        .fields()
        .iter()
        .map(|field| {
            Ok(format!(
                "{}:{}",
                json_string(field.name().name.as_ref()),
                lower_shape(field.shape().as_ref())?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let required = shape
        .fields()
        .iter()
        .map(|field| json_string(field.name().name.as_ref()))
        .collect::<Vec<_>>();
    Ok(format!(
        r#"{{"type":"object","properties":{{{}}},"required":[{}],"additionalProperties":false}}"#,
        properties.join(","),
        required.join(","),
    ))
}

fn lower_list_shape(shape: &ListShape) -> Result<String> {
    let prefix_items = shape
        .items()
        .iter()
        .map(|item| lower_shape(item.as_ref()))
        .collect::<Result<Vec<_>>>()?;
    let items = match shape.rest() {
        Some(rest) => lower_shape(rest.as_ref())?,
        None => "false".to_owned(),
    };
    let bounds = if shape.rest().is_none() {
        format!(
            r#","minItems":{},"maxItems":{}"#,
            shape.items().len(),
            shape.items().len()
        )
    } else {
        String::new()
    };
    Ok(format!(
        r#"{{"type":"array","prefixItems":[{}],"items":{}{}}}"#,
        prefix_items.join(","),
        items,
        bounds,
    ))
}

fn json_expr(expr: &Expr) -> Result<String> {
    Ok(match expr {
        Expr::Nil => "null".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::String(text) => json_string(text),
        Expr::Symbol(symbol) => json_string(&symbol.to_string()),
        Expr::List(items) | Expr::Vector(items) => {
            let items = items.iter().map(json_expr).collect::<Result<Vec<_>>>()?;
            format!("[{}]", items.join(","))
        }
        Expr::Map(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| {
                    let Expr::Symbol(symbol) = key else {
                        return Err(Error::Eval(
                            "shape_to_grammar exact map keys must be symbols".to_owned(),
                        ));
                    };
                    Ok(format!(
                        "{}:{}",
                        json_string(symbol.name.as_ref()),
                        json_expr(value)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            format!("{{{}}}", entries.join(","))
        }
        _ => {
            return Err(Error::Eval(
                "shape_to_grammar exact expr lowering only supports json-like forms".to_owned(),
            ));
        }
    })
}

fn json_string(text: &str) -> String {
    format!("{text:?}")
}

#[cfg(test)]
mod tests {
    use super::shape_to_grammar;
    use sim_kernel::Symbol;
    use sim_shape::{ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape};
    use std::sync::Arc;

    #[test]
    fn lowers_non_trivial_object_shape() {
        let grammar = shape_to_grammar(&FieldShape::anonymous(vec![
            FieldSpec::required(
                Symbol::new("name"),
                Arc::new(ExprKindShape::new(ExprKind::String)),
            ),
            FieldSpec::required(
                Symbol::new("versions"),
                Arc::new(ListShape::new(vec![
                    Arc::new(ExprKindShape::new(ExprKind::String)),
                    Arc::new(ExprKindShape::new(ExprKind::String)),
                ])),
            ),
        ]))
        .unwrap();

        assert!(grammar.contains(r#""type":"object""#));
        assert!(grammar.contains(r#""name":{"type":"string"}"#));
        assert!(grammar.contains(r#""versions":{"type":"array""#));
        assert!(grammar.contains(r#""additionalProperties":false"#));
    }
}
