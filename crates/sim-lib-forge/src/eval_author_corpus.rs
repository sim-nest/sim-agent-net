//! Deterministic corpus for offline FORGE authoring measurements.

use std::sync::Arc;

use sim_kernel::{Expr, NumberLiteral, Shape, Symbol};
use sim_shape::{
    ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape, OneOfShape, ShapeDefRef, ShapeDefs,
};

use crate::{AuthorCase, ContractCard, RankedContractCard};

pub(crate) fn standard_author_cases() -> Vec<AuthorCase> {
    vec![
        table_case(),
        record_case(),
        composition_case(),
        recursive_case(),
    ]
}

fn table_case() -> AuthorCase {
    let expected = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("status")),
            Expr::String("ready".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("entries")),
            Expr::Vector(vec![
                Expr::Vector(vec![Expr::String("alpha".to_owned()), int(1)]),
                Expr::Vector(vec![Expr::String("beta".to_owned()), int(2)]),
            ]),
        ),
    ]);
    AuthorCase::new(
        Symbol::qualified("forge-author-bench", "table-set-entries"),
        long_source(
            "Inspect the mutable project table, find the alpha row, set the status field to ready, \
             return the entries sorted by key, include a compact success record, avoid extra \
             commentary, avoid unrelated keys, and preserve the table operation boundary.",
        ),
        "Return the table update summary.",
        vec![bench_card(
            "table-set-entries",
            "set table status and return entries",
            "Map",
        )],
        Expr::Symbol(Symbol::qualified("shape", "fields")),
        record_shape(vec![
            ("status", Arc::new(ExprKindShape::new(ExprKind::String))),
            ("entries", Arc::new(ExprKindShape::new(ExprKind::Vector))),
        ]),
        expected,
    )
}

fn record_case() -> AuthorCase {
    let expected = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("name")),
            Expr::String("sensor".to_owned()),
        ),
        (Expr::Symbol(Symbol::new("enabled")), Expr::Bool(true)),
    ]);
    AuthorCase::new(
        Symbol::qualified("forge-author-bench", "small-record"),
        long_source(
            "Construct a small typed record for a sensor object with the canonical name, enabled \
             flag, and no spare fields; keep it as data, not prose, and return only the record \
             expected by the receiving checker.",
        ),
        "Return the sensor record.",
        vec![bench_card("small-record", "construct sensor record", "Map")],
        Expr::Symbol(Symbol::qualified("shape", "fields")),
        record_shape(vec![
            ("name", Arc::new(ExprKindShape::new(ExprKind::String))),
            ("enabled", Arc::new(ExprKindShape::new(ExprKind::Bool))),
        ]),
        expected,
    )
}

fn composition_case() -> AuthorCase {
    let expected = Expr::List(vec![
        Expr::Symbol(Symbol::qualified("forge", "compose")),
        Expr::Symbol(Symbol::qualified("export", "normalize")),
        Expr::Symbol(Symbol::qualified("export", "verify")),
    ]);
    AuthorCase::new(
        Symbol::qualified("forge-author-bench", "compose-two-exports"),
        long_source(
            "Build the composition form that applies the normalize export and then the verify \
             export, keep the operator and both export symbols in order, and return exactly the \
             compact form the dispatcher can inspect.",
        ),
        "Return the two-export composition form.",
        vec![bench_card(
            "compose-two-exports",
            "compose normalize then verify",
            "List",
        )],
        Expr::Symbol(Symbol::qualified("shape", "List")),
        Arc::new(ListShape::new(vec![
            Arc::new(ExprKindShape::new(ExprKind::Symbol)),
            Arc::new(ExprKindShape::new(ExprKind::Symbol)),
            Arc::new(ExprKindShape::new(ExprKind::Symbol)),
        ])),
        expected,
    )
}

fn recursive_case() -> AuthorCase {
    let expected = Expr::List(vec![int(1), Expr::List(vec![int(2), Expr::Nil])]);
    AuthorCase::new(
        Symbol::qualified("forge-author-bench", "recursive-node"),
        long_source(
            "Return a nested node expression where each node contains an integer head and either \
             another node or nil, keep the recursive shape intact, and do not flatten or narrate \
             the structure.",
        ),
        "Return the nested recursive node.",
        vec![bench_card(
            "recursive-node",
            "return integer node list",
            "Defs",
        )],
        Expr::Symbol(Symbol::qualified("shape", "Defs")),
        recursive_node_shape(),
        expected,
    )
}

fn long_source(task: &str) -> String {
    format!(
        "{task} Use the source-radar baseline wording with explicit constraints, \
         repeated context, detailed validation notes, and direct execution guidance so the \
         benchmark has a realistic raw prompt payload instead of a compact contract card."
    )
}

fn bench_card(name: &str, summary: &str, result_shape: &str) -> RankedContractCard {
    RankedContractCard {
        card: ContractCard {
            lib: Symbol::qualified("fixture", "author-bench"),
            export_kind: Symbol::qualified("export", "function"),
            symbol: Symbol::qualified("fixture", name),
            args_shape: Some(Expr::Symbol(Symbol::qualified("shape", "Any"))),
            result_shape: Some(Expr::Symbol(Symbol::qualified("shape", result_shape))),
            capability_symbols: Vec::new(),
            card_requires: None,
            summary: summary.to_owned(),
            example: None,
            partial: Vec::new(),
        },
        score: 10,
        reasons: vec!["author bench fixture".to_owned()],
    }
}

fn record_shape(fields: Vec<(&str, Arc<dyn Shape>)>) -> Arc<dyn Shape> {
    Arc::new(FieldShape::anonymous(
        fields
            .into_iter()
            .map(|(name, shape)| FieldSpec::required(Symbol::new(name), shape))
            .collect(),
    ))
}

fn recursive_node_shape() -> Arc<dyn Shape> {
    let node = Symbol::new("Node");
    Arc::new(ShapeDefs::new(
        Arc::new(ShapeDefRef::new(node.clone())),
        vec![(
            node.clone(),
            Arc::new(OneOfShape::new(vec![
                Arc::new(ExprKindShape::new(ExprKind::Nil)),
                Arc::new(ListShape::new(vec![
                    Arc::new(ExprKindShape::new(ExprKind::Number)),
                    Arc::new(ShapeDefRef::new(node)),
                ])),
            ])),
        )],
    ))
}

fn int(value: i64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("number", "i64"),
        canonical: value.to_string(),
    })
}
