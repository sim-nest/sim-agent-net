use std::sync::Arc;

use sim_kernel::{
    Cx, Expr, MatchScore, Result, ShapeDoc, ShapeMatch, Symbol, Value, shape::Shape,
    testing::bare_cx as cx,
};
use sim_lib_agent_runner_core::{
    FENCE_DATA_RULE, ModelRequest, OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA,
    OUTPUT_GRAMMAR_REQUIRED_EXTRA,
};
use sim_shape::{ExprKind, ExprKindShape};
use sim_value::access::field;

use crate::{
    AuthorTask, CONTRACT_PROJECTION_EXTRA, ContractCard, ContractProjection,
    ContractProjectionCaps, OUTPUT_GRAMMAR_GRAPH_EXTRA, RankedContractCard, ShapeQuery,
    author_model_request, estimate_prompt_tokens, project_contracts,
};

#[test]
fn projection_reduces_and_drops_without_exceeding_budget() {
    let cards = vec![
        ranked_card(
            "first",
            "set key",
            Some(Expr::Call {
                operator: Box::new(Expr::Symbol(Symbol::qualified("table", "set"))),
                args: vec![
                    Expr::Symbol(Symbol::new("table")),
                    Expr::Symbol(Symbol::new("alpha")),
                    Expr::String("value".to_owned()),
                ],
            }),
        ),
        ranked_card(
            "second",
            "return entries",
            Some(Expr::String("large".repeat(8))),
        ),
    ];
    let caps = ContractProjectionCaps::new(Symbol::qualified("codec", "lisp"), 12);

    let projection = project_contracts(&cards, &caps);

    assert!(projection.tokens <= caps.token_budget);
    assert_eq!(projection.included, 1);
    assert_eq!(projection.summary_only, 1);
    assert_eq!(projection.dropped, 1);
    assert!(projection.text.contains("contract: fixture/first"));
    assert!(!projection.text.contains("args-shape"));
    assert!(!projection.text.contains("fixture/second"));
    assert!(
        projection
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("summary only"))
    );
    assert!(
        projection
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("dropped fixture/second"))
    );
}

#[test]
fn strict_author_request_refuses_shapes_without_sg3_graph() {
    let mut cx = cx();
    let projection = projection("contract: fixture/echo\nsummary: return text");
    let task = AuthorTask {
        name: Symbol::qualified("forge", "unsupported"),
        goal: "Return checked data.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: 1,
        },
        return_shape_expr: Expr::Symbol(Symbol::qualified("fixture", "Unsupported")),
        return_shape: Arc::new(UnsupportedShape),
        strict_grammar: true,
    };

    let err = author_model_request(&mut cx, &task, &projection).unwrap_err();

    assert!(
        err.to_string()
            .contains("strict grammar cannot lower return shape")
    );
}

#[test]
fn author_request_carries_output_grammar_and_graph_metadata() {
    let mut cx = cx();
    let projection = projection("contract: fixture/echo\nsummary: return text");
    let task = AuthorTask {
        name: Symbol::qualified("forge", "echo"),
        goal: "Return a string answer.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: 1,
        },
        return_shape_expr: Expr::Symbol(Symbol::qualified("shape", "String")),
        return_shape: Arc::new(ExprKindShape::new(ExprKind::String)),
        strict_grammar: true,
    };

    let request = author_model_request(&mut cx, &task, &projection).unwrap();

    assert!(fenced_task_projection(&request).contains(FENCE_DATA_RULE));
    assert!(fenced_task_projection(&request).contains("contract: fixture/echo"));
    assert!(matches!(
        extra(&request, CONTRACT_PROJECTION_EXTRA),
        Some(Expr::String(text)) if text.contains(FENCE_DATA_RULE)
    ));
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_EXTRA),
        Some(&Expr::String(r#"{"type":"string"}"#.to_owned()))
    );
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_DIALECT_EXTRA),
        Some(&Expr::Symbol(Symbol::new("json-schema")))
    );
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_REQUIRED_EXTRA),
        Some(&Expr::Bool(true))
    );
    let graph = extra(&request, OUTPUT_GRAMMAR_GRAPH_EXTRA).unwrap();
    assert_eq!(
        field(graph, "kind"),
        Some(&Expr::Symbol(Symbol::qualified(
            "forge",
            "OutputGrammarGraph"
        )))
    );
    assert_eq!(
        field(graph, "root"),
        Some(&Expr::Symbol(Symbol::qualified(
            "grammar-production",
            "terminal"
        )))
    );
}

fn ranked_card(name: &str, summary: &str, example: Option<Expr>) -> RankedContractCard {
    RankedContractCard {
        card: ContractCard {
            lib: Symbol::qualified("fixture", "contracts"),
            export_kind: Symbol::qualified("export", "function"),
            symbol: Symbol::qualified("fixture", name),
            args_shape: Some(Expr::Symbol(Symbol::qualified("shape", "List"))),
            result_shape: Some(Expr::Symbol(Symbol::qualified("shape", "String"))),
            capability_symbols: vec![Symbol::qualified("capability", "table")],
            card_requires: None,
            summary: summary.to_owned(),
            example,
            partial: Vec::new(),
        },
        score: 10,
        reasons: vec!["fixture match".to_owned()],
    }
}

fn projection(text: &str) -> ContractProjection {
    ContractProjection {
        text: text.to_owned(),
        tokens: estimate_prompt_tokens(text),
        included: 1,
        summary_only: 0,
        dropped: 0,
        diagnostics: Vec::new(),
    }
}

fn fenced_task_projection(request: &ModelRequest) -> &str {
    match field(&request.task, "contract-projection") {
        Some(Expr::String(text)) => text,
        other => panic!("expected fenced contract projection, got {other:?}"),
    }
}

fn extra<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a Expr> {
    request.extra.iter().find_map(|(key, value)| {
        if *key == Expr::Symbol(Symbol::new(name)) {
            Some(value)
        } else {
            None
        }
    })
}

struct UnsupportedShape;

impl Shape for UnsupportedShape {
    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol::qualified("fixture", "UnsupportedShape"))
    }

    fn check_value(&self, _cx: &mut Cx, _value: Value) -> Result<ShapeMatch> {
        Ok(ShapeMatch::accept(MatchScore::exact(1)))
    }

    fn check_expr(&self, _cx: &mut Cx, _expr: &Expr) -> Result<ShapeMatch> {
        Ok(ShapeMatch::accept(MatchScore::exact(1)))
    }

    fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new("unsupported"))
    }
}
