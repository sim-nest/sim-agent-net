use sim_kernel::{Expr, Symbol};
use sim_shape::{GrammarDialect, GrammarGraph, Shape, shape_grammar_graph, shape_json_schema};

use crate::grammar::shape_to_grammar;

/// Model-request extension key for the required return codec.
pub const RETURN_CODEC_EXTRA: &str = "return-codec";
/// Model-request extension key for the normalized return shape expression.
pub const RETURN_SHAPE_EXTRA: &str = "return-shape";
/// Model-request extension key for a best-effort output grammar.
pub const OUTPUT_GRAMMAR_EXTRA: &str = "output-grammar";
/// Model-request extension key for the concrete grammar dialect.
pub const OUTPUT_GRAMMAR_DIALECT_EXTRA: &str = "output-grammar-dialect";
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
    /// Dialect used by [`Self::grammar`].
    pub grammar_dialect: Option<GrammarDialect>,
    /// Codec-neutral grammar source for provider-specific rendering.
    pub grammar_graph: Option<GrammarGraph>,
    /// Whether a runner that accepts grammars must enforce the grammar.
    pub required: bool,
}

impl OutputContract {
    /// Builds an output contract with an explicit grammar.
    pub fn new(codec: Symbol, shape_expr: Expr, grammar: Option<String>, required: bool) -> Self {
        let grammar_dialect = grammar.as_ref().map(|_| GrammarDialect::JsonSchema);
        Self {
            codec,
            shape_expr,
            grammar,
            grammar_dialect,
            grammar_graph: None,
            required,
        }
    }

    /// Builds an output contract, deriving a grammar when the shape supports it.
    pub fn for_shape(codec: Symbol, shape_expr: Expr, shape: &dyn Shape, required: bool) -> Self {
        let grammar_graph = shape_grammar_graph(shape).ok();
        let grammar = grammar_graph
            .as_ref()
            .and_then(|_| shape_json_schema(shape).ok())
            .or_else(|| shape_to_grammar(shape).ok());
        let grammar_dialect = grammar.as_ref().map(|_| GrammarDialect::JsonSchema);
        Self {
            codec,
            shape_expr,
            grammar,
            grammar_dialect,
            grammar_graph,
            required,
        }
    }

    /// Appends this contract to a [`ModelRequest`](crate::ModelRequest) `extra` map.
    pub fn into_extra_entries(self, extra: &mut Vec<(Expr, Expr)>) {
        extra.push(entry(RETURN_CODEC_EXTRA, Expr::Symbol(self.codec)));
        extra.push(entry(RETURN_SHAPE_EXTRA, self.shape_expr));
        if let Some(grammar) = self.grammar {
            extra.push(entry(OUTPUT_GRAMMAR_EXTRA, Expr::String(grammar)));
        }
        if let Some(dialect) = self.grammar_dialect {
            extra.push(entry(
                OUTPUT_GRAMMAR_DIALECT_EXTRA,
                Expr::Symbol(grammar_dialect_symbol(dialect)),
            ));
        }
        extra.push(entry(
            OUTPUT_GRAMMAR_REQUIRED_EXTRA,
            Expr::Bool(self.required),
        ));
    }
}

/// Returns the extension symbol used to name a grammar dialect.
pub fn grammar_dialect_symbol(dialect: GrammarDialect) -> Symbol {
    match dialect {
        GrammarDialect::JsonSchema => Symbol::new("json-schema"),
        GrammarDialect::Gbnf => Symbol::new("gbnf"),
        GrammarDialect::SExpr => Symbol::new("sexpr"),
    }
}

/// Parses a grammar dialect extension symbol.
pub fn grammar_dialect_from_symbol(symbol: &Symbol) -> Option<GrammarDialect> {
    match symbol.name.as_ref() {
        "json-schema" if symbol.namespace.is_none() => Some(GrammarDialect::JsonSchema),
        "gbnf" if symbol.namespace.is_none() => Some(GrammarDialect::Gbnf),
        "sexpr" if symbol.namespace.is_none() => Some(GrammarDialect::SExpr),
        _ => None,
    }
}

fn entry(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

#[cfg(test)]
mod tests {
    use super::{
        OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
        OutputContract, RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA,
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
            field(&extra, OUTPUT_GRAMMAR_DIALECT_EXTRA),
            Some(&Expr::Symbol(Symbol::new("json-schema")))
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
        assert!(field(&extra, OUTPUT_GRAMMAR_DIALECT_EXTRA).is_none());
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
