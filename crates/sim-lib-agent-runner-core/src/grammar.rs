use sim_kernel::Result;
use sim_shape::{Shape, shape_json_schema};

/// Lowers a SIM `shape` into a constrained-decoding grammar string.
///
/// The grammar can be handed to a model runner to constrain generation to
/// values that match `shape`.
pub fn shape_to_grammar(shape: &dyn Shape) -> Result<String> {
    shape_json_schema(shape)
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
