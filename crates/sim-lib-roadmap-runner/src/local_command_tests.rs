// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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

#[test]
fn command_export_matches_the_boot_entrypoint() {
    let manifest = LocalRoadmapRunnerLib::new().manifest();
    let expected = command_symbol();
    assert_eq!(
        expected.as_qualified_str(),
        sim_kernel::Symbol::qualified("cli", "main/roadmap").as_qualified_str()
    );
    assert_eq!(
        expected,
        sim_kernel::Symbol::qualified("cli", "main/roadmap")
    );
    assert!(manifest.exports.iter().any(|export| {
        matches!(export, Export::Function { symbol, .. } if symbol == &expected)
    }));
    let split = sim_kernel::Symbol::qualified("cli/main", "roadmap");
    assert_ne!(expected, split);
    assert!(
        manifest.exports.iter().all(|export| {
            !matches!(export, Export::Function { symbol, .. } if symbol == &split)
        })
    );
}

#[test]
fn loaded_command_binds_the_boot_entrypoint() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let entry = sim_kernel::Symbol::qualified("cli", "main/roadmap");
    let resolved = cx.registry().libs().iter().any(|lib| {
        lib.manifest.id == sim_kernel::Symbol::qualified("lib", "roadmap-runner")
            && lib.exports.iter().any(|record| {
                record.symbol == entry
                    && record.kind
                        == sim_kernel::ExportKind::named(sim_kernel::ExportKind::FUNCTION)
                    && matches!(record.state, sim_kernel::ExportState::Resolved { .. })
            })
    });
    assert!(resolved);
    let refused = cx
        .call_function(&entry, sim_kernel::Args::new(Vec::new()))
        .expect_err("envelope");
    assert!(
        refused.to_string().contains("missing roadmap envelope"),
        "{refused}"
    );
}

#[test]
fn bare_verb_reaches_the_identity_refusal() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let args = cx
        .factory()
        .list(vec![cx.factory().string("roadmap".to_owned()).unwrap()])
        .unwrap();
    let envelope = cx
        .factory()
        .table(vec![(sim_kernel::Symbol::new("args"), args)])
        .unwrap();
    let entry = sim_kernel::Symbol::qualified("cli", "main/roadmap");
    let refused = cx
        .call_function(&entry, sim_kernel::Args::new(vec![envelope]))
        .expect_err("identity");
    let text = refused.to_string();
    assert!(
        text.contains("missing content identity --conduct"),
        "{text}"
    );
    assert!(!text.contains("no loaded lib claims"), "{text}");
}

#[test]
fn status_with_identity_returns_the_stable_record_fields() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.load_lib(&LocalRoadmapRunnerLib::new()).unwrap();
    let text = [
        "roadmap",
        "status",
        "--conduct",
        "conduct/local",
        "--model-pick",
        "model/local",
        "--proof-catalog",
        "proof/local",
        "--runner-generation",
        "generation/local",
    ];
    let args = cx
        .factory()
        .list(
            text.iter()
                .map(|part| cx.factory().string((*part).to_owned()).unwrap())
                .collect(),
        )
        .unwrap();
    let envelope = cx
        .factory()
        .table(vec![(sim_kernel::Symbol::new("args"), args)])
        .unwrap();
    let entry = sim_kernel::Symbol::qualified("cli", "main/roadmap");
    let value = cx
        .call_function(&entry, sim_kernel::Args::new(vec![envelope]))
        .expect("status");
    let table = value.object().as_table_impl().expect("table");
    let field = |cx: &mut sim_kernel::Cx, name: &str| {
        let got = table.get(cx, sim_kernel::Symbol::new(name)).expect(name);
        match got.object().as_expr(cx).expect(name) {
            sim_kernel::Expr::String(text) => text,
            other => panic!("field {name} is {other:?}"),
        }
    };
    let outcome = field(&mut cx, "outcome");
    let detail = field(&mut cx, "detail");
    let acknowledged = {
        let got = table
            .get(&mut cx, sim_kernel::Symbol::new("journal-acknowledged"))
            .expect("journal-acknowledged");
        match got.object().as_expr(&mut cx).expect("journal-acknowledged") {
            sim_kernel::Expr::Bool(value) => value,
            other => panic!("field journal-acknowledged is {other:?}"),
        }
    };
    assert_eq!(outcome, "ok");
    assert_eq!(detail, "execution not started");
    assert!(acknowledged);
    let rendered = crate::local_emit::render_receipt(&LocalRoadmapReceipt {
        outcome,
        identity: LocalExecutionIdentity {
            execution: field(&mut cx, "execution"),
            conduct: field(&mut cx, "conduct"),
            model_pick: field(&mut cx, "model-pick"),
            proof_catalog: field(&mut cx, "proof-catalog"),
            runner_generation: field(&mut cx, "runner-generation"),
        },
        journal_acknowledged: acknowledged,
        detail,
    });
    assert_eq!(
        rendered,
        "#(roadmap/local-result-v1 (outcome \"ok\") (execution \"local/no-model\") (conduct \"conduct/local\") (model-pick \"model/local\") (proof-catalog \"proof/local\") (runner-generation \"generation/local\") (journal-acknowledged true) (detail \"execution not started\"))"
    );
}
