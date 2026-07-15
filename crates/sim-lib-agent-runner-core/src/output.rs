use sim_kernel::{Expr, Symbol};
use sim_shape::Shape;

use crate::grammar::shape_to_grammar;

/// Model-request extension key for the required return codec.
pub const RETURN_CODEC_EXTRA: &str = "return-codec";
/// Model-request extension key for the normalized return shape expression.
pub const RETURN_SHAPE_EXTRA: &str = "return-shape";
/// Model-request extension key for a best-effort output grammar.
pub const OUTPUT_GRAMMAR_EXTRA: &str = "output-grammar";
/// Model-request extension key marking whether the grammar is required.
pub const OUTPUT_GRAMMAR_REQUIRED_EXTRA: &str = "output-grammar-required";

/// Output obligations attached to a model request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputContract {
    /// Codec the receiver must use for the return value.
    pub codec: Symbol,
    /// Normalized shape expression describing the return value.
    pub shape_expr: Expr,
    /// Optional constrained-generation grammar derived from the shape.
    pub grammar: Option<String>,
    /// Whether a runner that accepts grammars must enforce the grammar.
    pub required: bool,
}

impl OutputContract {
    /// Builds an output contract with an explicit grammar.
    pub fn new(codec: Symbol, shape_expr: Expr, grammar: Option<String>, required: bool) -> Self {
        Self {
            codec,
            shape_expr,
            grammar,
            required,
        }
    }

    /// Builds an output contract, deriving a grammar when the shape supports it.
    pub fn for_shape(codec: Symbol, shape_expr: Expr, shape: &dyn Shape, required: bool) -> Self {
        Self::new(codec, shape_expr, shape_to_grammar(shape).ok(), required)
    }

    /// Appends this contract to a [`ModelRequest`](crate::ModelRequest) `extra` map.
    pub fn into_extra_entries(self, extra: &mut Vec<(Expr, Expr)>) {
        extra.push(entry(RETURN_CODEC_EXTRA, Expr::Symbol(self.codec)));
        extra.push(entry(RETURN_SHAPE_EXTRA, self.shape_expr));
        if let Some(grammar) = self.grammar {
            extra.push(entry(OUTPUT_GRAMMAR_EXTRA, Expr::String(grammar)));
        }
        extra.push(entry(
            OUTPUT_GRAMMAR_REQUIRED_EXTRA,
            Expr::Bool(self.required),
        ));
    }
}

fn entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

#[cfg(test)]
mod tests {
    use super::{
        OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA, OutputContract, RETURN_CODEC_EXTRA,
        RETURN_SHAPE_EXTRA,
    };
    use sim_kernel::{
        Cx, Expr, MatchScore, Result, ShapeDoc, ShapeMatch, Symbol, Value, shape::Shape,
    };
    use sim_shape::{ExprKind, ExprKindShape};

    struct NamedRecordShape;

    impl Shape for NamedRecordShape {
        fn symbol(&self) -> Option<Symbol> {
            Some(Symbol::qualified("bridge", "NamedRecord"))
        }

        fn check_value(&self, _cx: &mut Cx, _value: Value) -> Result<ShapeMatch> {
            Ok(ShapeMatch::accept(MatchScore::exact(1)))
        }

        fn check_expr(&self, _cx: &mut Cx, _expr: &Expr) -> Result<ShapeMatch> {
            Ok(ShapeMatch::accept(MatchScore::exact(1)))
        }

        fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
            Ok(ShapeDoc::new("named record"))
        }
    }

    #[test]
    fn supported_shape_attaches_grammar() {
        let contract = OutputContract::for_shape(
            Symbol::qualified("codec", "bridge"),
            Expr::Symbol(Symbol::qualified("shape", "String")),
            &ExprKindShape::new(ExprKind::String),
            true,
        );
        let mut extra = Vec::new();
        contract.into_extra_entries(&mut extra);

        assert_eq!(
            field(&extra, RETURN_CODEC_EXTRA),
            Some(&Expr::Symbol(Symbol::qualified("codec", "bridge")))
        );
        assert_eq!(
            field(&extra, RETURN_SHAPE_EXTRA),
            Some(&Expr::Symbol(Symbol::qualified("shape", "String")))
        );
        assert_eq!(
            field(&extra, OUTPUT_GRAMMAR_EXTRA),
            Some(&Expr::String(r#"{"type":"string"}"#.to_owned()))
        );
        assert_eq!(
            field(&extra, OUTPUT_GRAMMAR_REQUIRED_EXTRA),
            Some(&Expr::Bool(true))
        );
    }

    #[test]
    fn named_record_shape_omits_grammar_without_error() {
        let contract = OutputContract::for_shape(
            Symbol::qualified("codec", "bridge"),
            Expr::Symbol(Symbol::qualified("bridge", "NamedRecord")),
            &NamedRecordShape,
            true,
        );
        let mut extra = Vec::new();
        contract.into_extra_entries(&mut extra);

        assert!(field(&extra, OUTPUT_GRAMMAR_EXTRA).is_none());
        assert_eq!(
            field(&extra, RETURN_SHAPE_EXTRA),
            Some(&Expr::Symbol(Symbol::qualified("bridge", "NamedRecord")))
        );
        assert_eq!(
            field(&extra, OUTPUT_GRAMMAR_REQUIRED_EXTRA),
            Some(&Expr::Bool(true))
        );
    }

    fn field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
        entries.iter().find_map(|(key, value)| {
            if *key == Expr::Symbol(Symbol::new(name)) {
                Some(value)
            } else {
                None
            }
        })
    }
}
