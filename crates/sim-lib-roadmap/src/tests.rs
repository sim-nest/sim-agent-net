use sim_codec::{DecodeBudget, DecodeLimits, Input, decode_with_codec, encode_with_codec};
use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, Expr, HandleSeed, NoopEvalPolicy, ReadPolicy, Shape, Symbol,
};
use sim_lib_roadmap::*;
use std::{collections::BTreeMap, sync::Arc};

fn fields(entries: &[(&str, Expr)]) -> BTreeMap<Symbol, Expr> {
    entries
        .iter()
        .map(|(k, v)| (Symbol::new(*k), v.clone()))
        .collect()
}
fn nested_certificate() -> RoadmapValue {
    let leaf = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("phase")),
            Expr::String("leaf-3".into()),
        ),
        (
            Expr::Symbol(Symbol::new("limitations")),
            Expr::Vector(vec![Expr::String("scanner evidence only".into())]),
        ),
    ]);
    RoadmapValue::new(
        RoadmapValueKind::Certificate,
        fields(&[
            ("parent", Expr::String("root".into())),
            (
                "children",
                Expr::Vector(vec![Expr::Vector(vec![Expr::Vector(vec![leaf])])]),
            ),
            ("coverage", Expr::Bool(true)),
            (
                "limitations",
                Expr::Vector(vec![Expr::String(
                    "mixed exact and scanner evidence".into(),
                )]),
            ),
        ]),
    )
    .unwrap()
}

#[test]
fn lisp_and_json_preserve_three_level_certificate_semantics() {
    let expr = roadmap_value_to_expr(&nested_certificate());
    let json = sim_codec_json::expr_to_json(&expr);
    let mut budget = DecodeBudget::new(DecodeLimits::default());
    let json_back =
        sim_codec_json::json_to_expr(sim_kernel::CodecId(1), &json, &mut budget, 0).unwrap();
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(99),
    );
    let lib = sim_codec_lisp::LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lib).unwrap();
    let codec = Symbol::qualified("codec", "lisp");
    let lisp = encode_with_codec(&mut cx, &codec, &expr, Default::default())
        .unwrap()
        .into_text()
        .unwrap();
    let lisp_back =
        decode_with_codec(&mut cx, &codec, Input::Text(lisp), ReadPolicy::default()).unwrap();
    let original = roadmap_value_from_expr(&expr).unwrap();
    assert_eq!(
        roadmap_value_from_expr(&json_back).unwrap().semantic_id(),
        original.semantic_id()
    );
    assert_eq!(
        roadmap_value_from_expr(&lisp_back).unwrap().semantic_id(),
        original.semantic_id()
    );
}

#[test]
fn forged_id_fake_grounding_and_inconclusive_promise_are_rejected() {
    let mut forged = roadmap_value_to_expr(&nested_certificate());
    if let Expr::Extension { payload, .. } = &mut forged
        && let Expr::Map(fields) = payload.as_mut()
    {
        fields
            .iter_mut()
            .find(|(k, _)| matches!(k, Expr::Symbol(s) if s.name.as_ref()=="semantic-id"))
            .unwrap()
            .1 = Expr::String("forged".into());
    }
    assert!(roadmap_value_from_expr(&forged).is_err());
    assert!(
        RoadmapValue::new(
            RoadmapValueKind::Grounding,
            fields(&[
                ("deck", Expr::String("d".into())),
                ("roadmap", Expr::String("r".into())),
                ("verified", Expr::Bool(false))
            ])
        )
        .is_err()
    );
    assert!(
        RoadmapValue::new(
            RoadmapValueKind::Promise,
            fields(&[
                ("id", Expr::String("p".into())),
                ("conclusion", Expr::String("inconclusive".into()))
            ])
        )
        .is_err()
    );
}

#[test]
fn extensions_round_trip_unknown_structure_and_oversize_fail_before_copy() {
    let value = RoadmapValue::new(
        RoadmapValueKind::Explanation,
        fields(&[
            ("subject", Expr::String("p".into())),
            ("prose", Expr::String("why".into())),
            ("x-reviewer", Expr::String("human".into())),
        ]),
    )
    .unwrap();
    assert_eq!(
        roadmap_value_from_expr(&roadmap_value_to_expr(&value)).unwrap(),
        value
    );
    assert!(
        RoadmapValue::new(
            RoadmapValueKind::Explanation,
            fields(&[
                ("subject", Expr::String("p".into())),
                ("prose", Expr::String("why".into())),
                ("surprise", Expr::Nil)
            ])
        )
        .is_err()
    );
    let limits = RoadmapValueLimits {
        scalar_bytes: 3,
        ..Default::default()
    };
    assert!(
        RoadmapValue::with_limits(
            RoadmapValueKind::Explanation,
            fields(&[
                ("subject", Expr::String("p".into())),
                ("prose", Expr::String("too large".into()))
            ]),
            limits
        )
        .is_err()
    );
}

#[test]
fn cards_shapes_and_read_constructor_share_admission() {
    let value = nested_certificate();
    let expr = roadmap_value_to_expr(&value);
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(7),
    );
    let runtime = roadmap_value(&mut cx, value.clone()).unwrap();
    assert!(
        RoadmapValueShape::new(RoadmapValueKind::Certificate)
            .check_value(&mut cx, runtime)
            .unwrap()
            .accepted
    );
    assert!(
        RoadmapValueShape::any()
            .check_expr(&mut cx, &expr)
            .unwrap()
            .accepted
    );
    let card = roadmap_card(&mut cx, &value).unwrap();
    assert_eq!(
        card.object()
            .as_table_impl()
            .unwrap()
            .entries(&mut cx)
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn every_declared_kind_has_a_shape_symbol() {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(8),
    );
    for kind in ALL_KINDS {
        assert!(RoadmapValueShape::new(kind).describe(&mut cx).is_ok());
    }
}

#[test]
fn crate_has_no_effectful_or_document_ingestion_modules() {
    let manifest = include_str!("../Cargo.toml");
    let sources = [
        include_str!("lib.rs"),
        include_str!("value.rs"),
        include_str!("expr.rs"),
        include_str!("card.rs"),
        include_str!("shape.rs"),
        include_str!("projection.rs"),
        include_str!("read_construct.rs"),
    ]
    .join("\n");
    for forbidden in [
        "std::fs",
        "std::process",
        "reqwest",
        "tokio",
        "model",
        "scanner",
        "journal",
        "parser",
    ] {
        assert!(
            !sources.contains(forbidden),
            "forbidden capability marker: {forbidden}"
        );
        assert!(
            !manifest.contains(forbidden),
            "forbidden dependency marker: {forbidden}"
        );
    }
}
