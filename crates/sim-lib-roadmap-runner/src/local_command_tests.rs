use super::*;
#[test]
fn public_surface_has_only_local_verbs_and_explicit_shapes() {
    let m = LocalRoadmapRunnerLib::new().manifest();
    for verb in LOCAL_ROADMAP_VERBS {
        assert!(m.exports.contains(&Export::Function {
            symbol: op_symbol(verb),
            function_id: None,
        }));
        assert!(m.exports.contains(&Export::Shape {
            symbol: shape_symbol(verb, "Args"),
            shape_id: None,
        }));
        assert!(m.exports.contains(&Export::Shape {
            symbol: shape_symbol(verb, "Result"),
            shape_id: None,
        }));
    }
    let text = format!("{:?}", m.exports);
    for forbidden in FORBIDDEN {
        assert!(!text.contains(&format!("roadmap/{forbidden}")));
    }
}
#[test]
fn mutation_needs_disposable_checkout_and_token() {
    let p = PublicLocalRoadmapPort::default();
    let r = parse_cli(
        "run",
        &[
            "--conduct",
            "sha:c",
            "--model-pick",
            "sha:m",
            "--proof-catalog",
            "sha:p",
            "--runner-generation",
            "sha:g",
        ]
        .map(str::to_owned),
    )
    .unwrap();
    assert!(
        p.invoke(&r, GenerationHandle::acquire("sha:g"))
            .unwrap_err()
            .contains("disposable")
    );
}
#[test]
fn observe_and_replay_are_effect_bounded_and_identity_complete() {
    let p = PublicLocalRoadmapPort::default();
    let r = parse_cli(
        "run",
        &[
            "--observe",
            "--execution",
            "e",
            "--conduct",
            "sha:c",
            "--model-pick",
            "sha:m",
            "--proof-catalog",
            "sha:p",
            "--runner-generation",
            "sha:g",
        ]
        .map(str::to_owned),
    )
    .unwrap();
    let h = GenerationHandle::acquire("sha:g");
    let retained = h.clone();
    let receipt = p.invoke(&r, h).unwrap();
    assert_eq!(retained.retained(), 1);
    assert!(receipt.journal_acknowledged);
    assert_eq!(receipt.identity, r.identity);
    let mut replay = r;
    replay.verb = "replay".into();
    assert!(
        p.invoke(&replay, retained)
            .unwrap()
            .detail
            .contains("replayed")
    );
}
