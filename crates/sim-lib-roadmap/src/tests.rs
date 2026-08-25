// conformance: capability-free roadmap grounding and refinement operations.

use sim_codec::{DecodeBudget, DecodeLimits, Input, decode_with_codec, encode_with_codec};
use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, Expr, HandleSeed, Lib, NoopEvalPolicy, ReadPolicy, Shape,
    Symbol,
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

fn operation_fixture(kind: RoadmapValueKind, entries: &[(&str, Expr)]) -> RoadmapValue {
    RoadmapValue::new(kind, fields(entries)).unwrap()
}

#[test]
fn pure_operations_ground_plan_diff_refine_render_and_explain() {
    let deck = operation_fixture(
        RoadmapValueKind::SourceDeck,
        &[
            (
                "repositories",
                Expr::Vector(vec![Expr::String("sim-agent-net".into())]),
            ),
            (
                "evidence",
                Expr::Vector(vec![Expr::String("evidence/source-line".into())]),
            ),
            (
                "limitations",
                Expr::Vector(vec![Expr::String("unresolved source witness".into())]),
            ),
        ],
    );
    let phase = Expr::Map(vec![(
        Expr::Symbol(Symbol::new("id")),
        Expr::String("phase/root/child/leaf".into()),
    )]);
    let roadmap = operation_fixture(
        RoadmapValueKind::Roadmap,
        &[
            ("revision", Expr::String("r1".into())),
            ("phases", Expr::Vector(vec![phase])),
        ],
    );
    let grounding = apply_operation("ground", &[deck, roadmap.clone()]).unwrap();
    assert!(format!("{:?}", grounding.fields()).contains("unresolved source witness"));
    let plan = apply_operation("plan", std::slice::from_ref(&roadmap)).unwrap();
    let observations = plan.fields().get(&Symbol::new("observations")).unwrap();
    for required in [
        "tree",
        "complete-ready-set",
        "promises",
        "derived-profiles",
        "atomicity",
        "aggregate-completion",
    ] {
        assert!(format!("{observations:?}").contains(required), "{required}");
    }
    assert!(
        apply_operation("diff", &[roadmap.clone(), roadmap.clone()])
            .unwrap()
            .fields()[&Symbol::new("changes")]
            .canonical_eq(&Expr::Vector(vec![]))
    );
    let proposal = operation_fixture(
        RoadmapValueKind::Refinement,
        &[
            ("parent", Expr::String("phase/root".into())),
            (
                "children",
                Expr::Vector(vec![Expr::String("phase/root/child".into())]),
            ),
        ],
    );
    assert_eq!(
        apply_operation("refine", &[roadmap.clone(), grounding.clone(), proposal])
            .unwrap()
            .kind(),
        RoadmapValueKind::Certificate
    );
    assert!(apply_operation("render", std::slice::from_ref(&roadmap)).is_ok());
    let subject = operation_fixture(
        RoadmapValueKind::Explanation,
        &[
            ("subject", Expr::String("phase/root".into())),
            ("prose", Expr::String("subject".into())),
        ],
    );
    assert!(apply_operation("explain", &[roadmap, subject]).is_ok());
}

#[test]
fn roadmap_library_exports_exact_operations_and_command_without_capabilities() {
    let manifest = RoadmapLib::new().manifest();
    assert!(manifest.capabilities.is_empty() && manifest.requires.is_empty());
    let symbols: Vec<_> = manifest
        .exports
        .iter()
        .map(|export| match export {
            sim_kernel::Export::Function { symbol, .. } => symbol.to_string(),
            _ => String::new(),
        })
        .collect();
    for operation in ROADMAP_OPERATIONS {
        assert!(symbols.contains(&format!("roadmap/{operation}")));
    }
    assert!(symbols.contains(&"cli/main/roadmap".to_owned()));
}

#[test]
fn crate_has_no_effectful_ingestion_dependencies() {
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
        "scanner",
        "journal",
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

#[test]
fn v3_import_separates_observations_and_preserves_spans() {
    let referenced = b"### [ ] OTHER.1 - Source\n".to_vec();
    let documents = BTreeMap::from([("source.md".to_owned(), referenced)]);
    let source = "# Plan\n\nSTATE: ACTIVE\nNEXT: P.1\n\n### [ ] P.1 - Build it\nSOURCE_PHASE: source.md#OTHER.1\nDEPENDS_ON: P.0\nStatus: active\n\nTasks:\n\n- [x] parsed task\n\nSome legacy prose.\n\n```sh\necho visible\n```\n";
    let imported = V3Importer::new(&documents)
        .import("plan.md", source)
        .unwrap();
    assert_eq!(imported.observation.state.as_deref(), Some("ACTIVE"));
    assert_eq!(imported.observation.next.as_deref(), Some("P.1"));
    assert_eq!(imported.observation.checked_tasks["P.1"], vec![1]);
    let phase = &imported.roadmap.root.children[0];
    assert_eq!(phase.dependencies[0].target, "P.0");
    assert_eq!(phase.checkpoints[0].text, "parsed task");
    assert!(phase.span.end > phase.span.start);
    assert!(phase.guides.iter().all(|guide| !guide.grounded));
    assert!(
        phase
            .guides
            .iter()
            .any(|guide| guide.language.as_deref() == Some("sh"))
    );
    let before = imported.roadmap.semantic_id();
    let mut changed = imported;
    changed.observation.state = Some("DONE".into());
    changed.observation.checked_tasks.clear();
    assert_eq!(before, changed.roadmap.semantic_id());
    changed.roadmap.root.children[0].title.push('!');
    assert_ne!(before, changed.roadmap.semantic_id());
}

#[test]
fn references_are_caller_owned_content_pinned_and_fail_closed() {
    let docs = BTreeMap::from([("source.md".to_owned(), b"source".to_vec())]);
    let base = "# Plan\n\n### [ ] P.1 - Build\nSOURCE_PHASE: {}\n";
    let imported = V3Importer::new(&docs)
        .import("plan.md", &base.replace("{}", "source.md#S.1"))
        .unwrap();
    assert!(
        imported.roadmap.root.children[0].origins[0]
            .content_id
            .starts_with("fnv1a64:")
    );
    for reference in ["../source.md#S.1", "/source.md#S.1", "missing.md#S.1"] {
        assert!(
            V3Importer::new(&docs)
                .import("plan.md", &base.replace("{}", reference))
                .is_err()
        );
    }
    assert!(
        V3Importer::new(&docs)
            .import(
                "plan.md",
                &base.replace("{}", "source.md@fnv1a64:0000000000000000#S.1"),
            )
            .is_err()
    );
    let duplicate = "# Plan\n\n### [ ] P.1 - Build\nSOURCE_PHASE: source.md#S.1, source.md#S.1\n";
    assert!(V3Importer::new(&docs).import("plan.md", duplicate).is_err());
}

#[test]
fn catalogs_use_config_shape_and_require_argv_commands() {
    let ok = "name = \"workspace\"\ncommand = [\"cargo\", \"test\"]\n";
    assert!(import_catalogs(ok, ok, ok, ok).is_ok());
    let shell = "command = \"cargo test && publish\"\n";
    assert!(import_catalogs(shell, ok, ok, ok).is_err());
}

#[test]
fn native_render_is_stable_visible_and_v3_reports_all_loss_paths() {
    let docs = BTreeMap::new();
    let source = "# Plan\n\n### [ ] P.1 - Build\n\nProse.\n";
    let imported = V3Importer::new(&docs).import("plan.md", source).unwrap();
    let native = render_native(&imported.roadmap);
    assert_eq!(native, render_native(&imported.roadmap));
    assert!(native.contains("UNGROUNDED LEGACY GUIDE"));
    assert!(!native.contains("Status:") && !native.contains("STATE:"));
    assert!(render_v3(&imported.roadmap).is_ok());
    let mut recursive = imported.roadmap;
    let child = recursive.root.children[0].clone();
    recursive.root.children[0].children.push(child);
    recursive.root.children[0].guides[0].grounded = true;
    let losses = render_v3(&recursive).unwrap_err();
    assert_eq!(losses.losses.len(), 2);
    assert!(
        losses
            .losses
            .iter()
            .all(|loss| loss.path.starts_with("root.phases"))
    );
}

#[test]
fn public_v3_corpus_covers_all_legacy_shapes_and_is_observation_stable() {
    let ordinary = include_str!("../qualification/v3/ordinary.md");
    let source = include_str!("../qualification/v3/source.md");
    let review = include_str!("../qualification/v3/review.md");
    let resource = include_str!("../qualification/v3/resource.md");
    let closeout = include_str!("../qualification/v3/merged-closeout.md");
    let completed = include_str!("../qualification/v3/completed.md");
    let malformed = include_str!("../qualification/v3/malformed.md");
    let documents = BTreeMap::from([
        ("ordinary.md".into(), ordinary.as_bytes().to_vec()),
        ("source.md".into(), source.as_bytes().to_vec()),
        ("review.md".into(), review.as_bytes().to_vec()),
    ]);
    let importer = V3Importer::new(&documents);
    for (path, text) in [
        ("ordinary.md", ordinary),
        ("source.md", source),
        ("review.md", review),
        ("resource.md", resource),
        ("merged-closeout.md", closeout),
        ("completed.md", completed),
    ] {
        let imported = importer.import(path, text).unwrap();
        assert_eq!(
            imported.roadmap.semantic_id(),
            imported.roadmap.semantic_id()
        );
        assert!(
            imported
                .roadmap
                .root
                .children
                .iter()
                .all(|p| p.span.end <= text.len())
        );
    }
    assert!(importer.import("malformed.md", malformed).is_err());

    let imported = importer.import("completed.md", completed).unwrap();
    let id = imported.roadmap.semantic_id();
    let active = completed
        .replace("COMPLETE", "ACTIVE")
        .replace("[x]", "[ ]");
    assert_eq!(
        id,
        importer
            .import("completed.md", &active)
            .unwrap()
            .roadmap
            .semantic_id()
    );

    let oversized = format!("# Bomb\n\n### [ ] B.1 - Bomb\n\n{}", "x".repeat(1_000_001));
    assert!(importer.import("bomb.md", &oversized).is_err());
}

#[test]
fn native_three_level_corpus_is_public_and_python_independent() {
    let corpus = include_str!("../qualification/v3/native-three-level.md");
    assert!(corpus.contains("level-1") && corpus.contains("level-2") && corpus.contains("level-3"));
    for forbidden in [
        "docs/workbench",
        "sim-private",
        "roadmap_engine_3",
        "python",
    ] {
        assert!(!corpus.to_ascii_lowercase().contains(forbidden));
    }
}
