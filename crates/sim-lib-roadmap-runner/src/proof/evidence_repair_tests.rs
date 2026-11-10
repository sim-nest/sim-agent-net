// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Negative evidence attachment: subject, proof body, and source identity.

use std::collections::{BTreeMap, BTreeSet};

use sim_conformance_core::EvidenceGrade;
use sim_lib_exec::SandboxLimits;

use super::evidence_fixture::{
    GradeIds, SOURCE, admit, anchor_leaf, catalog_for, catalog_with_closure, claim, closure_for,
    current, graded_catalog, install_release, refused, sid,
};
use super::{EvidenceClaim, EvidenceDecision};
use crate::{CommandProof, ProofLeaf, ProofOutputId, StructuredExpectation};

fn bootstrap(ids: &GradeIds) -> EvidenceClaim {
    claim(ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![])
}

#[test]
fn a_different_subject_or_scope_does_not_attach() {
    let (mut catalog, ids) = graded_catalog();
    let mut subject = bootstrap(&ids);
    subject.subject = sid("subject/other");
    assert!(refused(
        catalog.admit_evidence(subject, current),
        "unrelated-invocation"
    ));
    let mut scope = bootstrap(&ids);
    scope.scope = sid("scope/other");
    assert!(refused(
        catalog.admit_evidence(scope, current),
        "unrelated-invocation"
    ));
    let mut environment = bootstrap(&ids);
    environment.environment = "environment/other".into();
    assert!(refused(
        catalog.admit_evidence(environment, current),
        "unrelated-invocation"
    ));
}

#[test]
fn a_changed_proof_body_keeps_a_distinct_key() {
    let projections = ["projection/body".to_owned()];
    let (mut original_catalog, original_ids) = graded_catalog();
    let original = admit(
        &mut original_catalog,
        &original_ids,
        EvidenceGrade::Bootstrap,
        "",
        None,
    );
    let changed = ProofLeaf::ArtifactEquality {
        name: "anchor".into(),
        left: b"same".to_vec(),
        right: b"other".to_vec(),
    };
    let (mut changed_catalog, changed_ids) = catalog_for(
        changed.clone(),
        &projections,
        SOURCE,
        "subject/lantern",
        true,
    );
    let changed_admitted = changed_catalog
        .admit_evidence(bootstrap(&changed_ids), current)
        .unwrap();
    assert_ne!(original.key, changed_admitted.key);

    let closure = closure_for(
        &anchor_leaf(),
        &projections,
        SOURCE,
        "subject/lantern",
        "scope/evidence",
        true,
    );
    let (mut mismatched, mismatch_ids) = catalog_with_closure(changed, closure, "subject/lantern");
    assert!(refused(
        mismatched.admit_evidence(bootstrap(&mismatch_ids), current),
        "unrelated-invocation"
    ));
}

#[test]
fn a_command_proof_does_not_share_an_artifact_key() {
    let (mut artifact_catalog, artifact_ids) = graded_catalog();
    let artifact = admit(
        &mut artifact_catalog,
        &artifact_ids,
        EvidenceGrade::Bootstrap,
        "",
        None,
    );
    let (mut command_catalog, command_ids) = catalog_for(
        anchor_command(),
        &["projection/body".into()],
        SOURCE,
        "subject/lantern",
        true,
    );
    let command = command_catalog
        .admit_evidence(bootstrap(&command_ids), current)
        .unwrap();
    assert_ne!(artifact.key, command.key);
}

#[test]
fn a_label_only_invocation_reports_changed_source_semantic() {
    let leaf = anchor_leaf();
    let closure = closure_for(
        &leaf,
        &["projection/body".into()],
        SOURCE,
        "subject/lantern",
        "scope/evidence",
        false,
    );
    let (mut catalog, ids) = catalog_with_closure(leaf, closure, "subject/lantern");
    assert!(refused(
        catalog.admit_evidence(bootstrap(&ids), current),
        "changed-source-semantic"
    ));
}

#[test]
fn a_changed_checksum_does_not_reuse_the_bound_receipt() {
    let drifted = SOURCE.replacen("aaaa1111", "ffff9999", 1);
    let (mut catalog, ids) = graded_catalog();
    let mut stale = bootstrap(&ids);
    stale.source_text = drifted.clone();
    assert!(refused(
        catalog.admit_evidence(stale, current),
        "unrelated-invocation"
    ));

    let (mut fresh_catalog, fresh_ids) = catalog_for(
        anchor_leaf(),
        &["projection/body".into()],
        &drifted,
        "subject/lantern",
        true,
    );
    let mut fresh = bootstrap(&fresh_ids);
    fresh.source_text = drifted;
    let fresh_admitted = fresh_catalog.admit_evidence(fresh, current).unwrap();
    let original = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    assert_ne!(original.key, fresh_admitted.key);
}

#[test]
fn an_unselected_definition_does_not_change_the_key() {
    let extra = format!("{SOURCE}generated resolver/lantern src/other.rs 4 eeee5555\n");
    let (mut catalog, ids) = graded_catalog();
    let baseline = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let mut wider = bootstrap(&ids);
    wider.source_text = extra;
    let again = catalog.admit_evidence(wider, current).unwrap();
    assert_eq!(again.key, baseline.key);
    assert_eq!(again.id, baseline.id);
}

#[test]
fn a_bound_long_projection_reuses_only_its_own_receipt() {
    let wide = "p".repeat(20_000);
    let other = "q".repeat(20_000);
    let (mut catalog, ids) = catalog_for(
        anchor_leaf(),
        std::slice::from_ref(&wide),
        SOURCE,
        "subject/lantern",
        true,
    );
    let mut first_claim = bootstrap(&ids);
    first_claim.projections = vec![wide.clone()];
    let first = catalog.admit_evidence(first_claim, current).unwrap();
    let mut repeat = bootstrap(&ids);
    repeat.projections = vec![wide];
    let again = catalog.admit_evidence(repeat, current).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.key, first.key);

    let mut mismatch = bootstrap(&ids);
    mismatch.projections = vec![other.clone()];
    assert!(refused(
        catalog.admit_evidence(mismatch, current),
        "unrelated-invocation"
    ));

    let (mut other_catalog, other_ids) = catalog_for(
        anchor_leaf(),
        std::slice::from_ref(&other),
        SOURCE,
        "subject/lantern",
        true,
    );
    let mut other_claim = bootstrap(&other_ids);
    other_claim.projections = vec![other];
    let other_admitted = other_catalog.admit_evidence(other_claim, current).unwrap();
    assert_ne!(other_admitted.key, first.key);
    assert_ne!(other_admitted.id, first.id);
}

#[test]
fn a_different_subject_or_scope_does_not_reuse_another_receipt() {
    let (catalog, ids) = graded_catalog();
    let mut catalog = catalog;
    let lantern = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let projections = ["projection/body".to_owned()];
    let subject_closure = closure_for(
        &anchor_leaf(),
        &projections,
        SOURCE,
        "subject/other",
        "scope/evidence",
        true,
    );
    let (mut catalog, subject_receipt) =
        install_release(catalog, "subject/other", "scope/evidence", subject_closure);
    let mut other_subject = bootstrap(&ids);
    other_subject.subject = sid("subject/other");
    other_subject.checker_receipt = subject_receipt;
    let other_subject = catalog.admit_evidence(other_subject, current).unwrap();
    assert_ne!(lantern.key, other_subject.key);
    assert_eq!(
        catalog.decide_evidence(&lantern.key, EvidenceGrade::Release, current),
        EvidenceDecision::Recheck { reason: "grade" }
    );
    assert_eq!(
        catalog.decide_evidence(&other_subject.key, EvidenceGrade::Release, current),
        EvidenceDecision::Reuse {
            receipt: other_subject.id
        }
    );

    let scope_closure = closure_for(
        &anchor_leaf(),
        &projections,
        SOURCE,
        "subject/lantern",
        "scope/other",
        true,
    );
    let (mut catalog, scope_receipt) =
        install_release(catalog, "subject/lantern", "scope/other", scope_closure);
    let mut other_scope = bootstrap(&ids);
    other_scope.scope = sid("scope/other");
    other_scope.checker_receipt = scope_receipt;
    let other_scope = catalog.admit_evidence(other_scope, current).unwrap();
    assert_ne!(lantern.key, other_scope.key);
    assert_ne!(other_subject.key, other_scope.key);
    assert_eq!(
        catalog.decide_evidence(&lantern.key, EvidenceGrade::Release, current),
        EvidenceDecision::Recheck { reason: "grade" }
    );
}

#[test]
fn command_bodies_do_not_share_a_key_when_their_text_joins_collide() {
    let allowed = BTreeSet::from(["A".to_owned(), "A=B".to_owned()]);
    let mut left_env = BTreeMap::new();
    left_env.insert("A".to_owned(), "B=C".to_owned());
    let mut right_env = BTreeMap::new();
    right_env.insert("A=B".to_owned(), "C".to_owned());
    let projections = ["projection/body".to_owned()];
    let left = command_leaf(left_env, allowed.clone(), Some("scratch:proof".into()));
    let right = command_leaf(right_env, allowed, Some("scratch:proof".into()));
    let (mut left_catalog, left_ids) =
        catalog_for(left.clone(), &projections, SOURCE, "subject/lantern", true);
    let (mut right_catalog, right_ids) =
        catalog_for(right.clone(), &projections, SOURCE, "subject/lantern", true);
    let left_admitted = left_catalog
        .admit_evidence(bootstrap(&left_ids), current)
        .unwrap();
    let right_admitted = right_catalog
        .admit_evidence(bootstrap(&right_ids), current)
        .unwrap();
    assert_ne!(left_admitted.key, right_admitted.key);
    let closure = closure_for(
        &left,
        &projections,
        SOURCE,
        "subject/lantern",
        "scope/evidence",
        true,
    );
    let (mut mismatched, mismatch_ids) = catalog_with_closure(right, closure, "subject/lantern");
    assert!(refused(
        mismatched.admit_evidence(bootstrap(&mismatch_ids), current),
        "unrelated-invocation"
    ));

    let absent = command_leaf(BTreeMap::new(), BTreeSet::new(), None);
    let empty = command_leaf(BTreeMap::new(), BTreeSet::new(), Some(String::new()));
    let (mut absent_catalog, absent_ids) =
        catalog_for(absent, &projections, SOURCE, "subject/lantern", true);
    let (mut empty_catalog, empty_ids) =
        catalog_for(empty, &projections, SOURCE, "subject/lantern", true);
    let absent_admitted = absent_catalog
        .admit_evidence(bootstrap(&absent_ids), current)
        .unwrap();
    let empty_admitted = empty_catalog
        .admit_evidence(bootstrap(&empty_ids), current)
        .unwrap();
    assert_ne!(absent_admitted.key, empty_admitted.key);
}

fn anchor_command() -> ProofLeaf {
    command_leaf(
        BTreeMap::new(),
        BTreeSet::new(),
        Some("scratch:proof".into()),
    )
}

fn command_leaf(
    environment: BTreeMap<String, String>,
    allowed: BTreeSet<String>,
    scratch: Option<String>,
) -> ProofLeaf {
    ProofLeaf::command(CommandProof {
        name: "anchor".into(),
        effect_id: "effect-anchor".into(),
        program: "tool:harmless".into(),
        argv: vec!["--literal".into()],
        working_directory: "/source".into(),
        environment,
        allowed_environment_keys: allowed,
        source_mount: "checkout:exact-head".into(),
        scratch_mount: scratch,
        source_read_only: true,
        limits: SandboxLimits {
            cpu_seconds: 1,
            memory_bytes: 32 * 1024 * 1024,
            wall_time_ms: 100,
            process_count: 2,
            file_count: 2,
            file_bytes: 1024,
            output_bytes: 1024,
            stdin_bytes: 1,
        },
        expected: StructuredExpectation {
            stdout: ProofOutputId::sha256(b"ok"),
            exit_code: 0,
        },
    })
}
