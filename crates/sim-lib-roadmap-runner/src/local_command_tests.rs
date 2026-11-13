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
