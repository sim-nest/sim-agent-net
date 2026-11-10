// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;

use sim_conformance_core::{CheckerReceiptId, EvidenceGrade, RevocationStatus};

use super::evidence_fixture::{SOURCE, admit, claim, current, graded_catalog, refused, row, sid};
use super::{EvidenceDecision, discover_definitions};

#[test]
fn sealed_text_discovers_the_four_definition_classes() {
    let found = discover_definitions(SOURCE).unwrap();
    let classes = found.iter().map(|row| row.class).collect::<BTreeSet<_>>();
    assert_eq!(
        classes,
        BTreeSet::from(["generate", "package", "standalone", "workspace"])
    );
    let generated = row(&found, "generate");
    assert_eq!(generated.name, "src/generated.rs");
    assert_ne!(generated.semantic, generated.storage);
    assert_eq!(row(&found, "workspace").domain, "resolver/lantern");
    assert_eq!(row(&found, "standalone").name, "widget");
}

#[test]
fn grade_promotion_keeps_the_key_and_reuses_the_required_grade() {
    let (mut catalog, ids) = graded_catalog();
    let bootstrap = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let checked = admit(
        &mut catalog,
        &ids,
        EvidenceGrade::Reproducible,
        "sample-1",
        None,
    );
    let published = admit(&mut catalog, &ids, EvidenceGrade::Release, "sample-2", None);
    assert_eq!(bootstrap.key, checked.key);
    assert_eq!(checked.key, published.key);
    assert_ne!(bootstrap.id, checked.id);
    assert_eq!(checked.grade, EvidenceGrade::Reproducible);
    let again = admit(
        &mut catalog,
        &ids,
        EvidenceGrade::Reproducible,
        "different-telemetry",
        None,
    );
    assert_eq!(again.id, checked.id);
    assert_eq!(
        catalog.decide_evidence(&bootstrap.key, EvidenceGrade::Bootstrap, current),
        EvidenceDecision::Reuse {
            receipt: bootstrap.id,
        }
    );
    assert_eq!(
        catalog.decide_evidence(&bootstrap.key, EvidenceGrade::Reproducible, current),
        EvidenceDecision::Reuse {
            receipt: checked.id,
        }
    );
    assert_eq!(
        catalog.decide_evidence(&bootstrap.key, EvidenceGrade::Release, current),
        EvidenceDecision::Reuse {
            receipt: published.id,
        }
    );
}

#[test]
fn a_weaker_grade_asks_for_a_real_check() {
    let (mut catalog, ids) = graded_catalog();
    let admitted = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    assert_eq!(
        catalog.decide_evidence(&admitted.key, EvidenceGrade::Reproducible, current),
        EvidenceDecision::Recheck { reason: "grade" }
    );
}

#[test]
fn requested_grade_cannot_convert_the_checker_grade() {
    let (mut catalog, ids) = graded_catalog();
    let error = catalog
        .admit_evidence(
            claim(
                &ids,
                EvidenceGrade::Bootstrap,
                "",
                Some(EvidenceGrade::Release),
                vec![],
                vec![],
            ),
            current,
        )
        .expect_err("conversion");
    assert!(matches!(
        error,
        crate::ProofError::Refused("grade-conversion")
    ));
}

#[test]
fn wrong_domain_missing_object_storage_and_policy_refuse() {
    let (mut catalog, ids) = graded_catalog();
    let found = discover_definitions(SOURCE).unwrap();
    let workspace = row(&found, "workspace");
    let mut wrong = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    wrong.domains = vec!["resolver/other".into()];
    assert!(refused(
        catalog.admit_evidence(wrong, current),
        "wrong-domain"
    ));
    let mut missing = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    missing.members = vec!["generate:resolver/lantern:missing.rs".into()];
    assert!(refused(
        catalog.admit_evidence(missing, current),
        "missing-object"
    ));
    let mut storage = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    storage.members = vec![workspace.storage.clone()];
    assert!(refused(
        catalog.admit_evidence(storage, current),
        "storage-id-crossing"
    ));
    let mut policy = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    policy.policies.push(sid("policy/unrelated"));
    assert!(refused(
        catalog.admit_evidence(policy, current),
        "unrelated-policy"
    ));
    let mut projected = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    projected.projections = vec![workspace.storage.clone()];
    assert!(refused(
        catalog.admit_evidence(projected, current),
        "storage-id-crossing"
    ));
}

#[test]
fn revoked_support_and_unknown_revocation_block_reuse() {
    let (mut catalog, ids) = graded_catalog();
    let support = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let checked = admit(&mut catalog, &ids, EvidenceGrade::Reproducible, "", None);
    let dependent = claim(
        &ids,
        EvidenceGrade::Reproducible,
        "",
        None,
        vec![support.id.clone()],
        vec![],
    );
    let dependent = catalog.admit_evidence(dependent, current).unwrap();
    assert_ne!(dependent.key, checked.key);
    let support_checker = ids[&EvidenceGrade::Bootstrap].clone();
    let decision = catalog.decide_evidence(&dependent.key, EvidenceGrade::Reproducible, |id| {
        if id == &support_checker {
            RevocationStatus::Revoked
        } else {
            RevocationStatus::Current
        }
    });
    assert_eq!(
        decision,
        EvidenceDecision::Refuse {
            reason: "revoked-support",
        }
    );
    assert!(refused(
        catalog.admit_evidence(
            claim(&ids, EvidenceGrade::Release, "late", None, vec![], vec![]),
            |_| RevocationStatus::Unknown,
        ),
        "unknown-revocation"
    ));
}

#[test]
fn a_long_projection_does_not_attach_to_another_receipt() {
    let (mut catalog, ids) = graded_catalog();
    let mut wide = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    wide.projections = vec!["p".repeat(20_000)];
    assert!(refused(
        catalog.admit_evidence(wide, current),
        "unrelated-invocation"
    ));
}

#[test]
fn revoked_indirect_support_blocks_admission_and_reuse() {
    let (mut catalog, ids) = graded_catalog();
    let inner = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let middle = claim(
        &ids,
        EvidenceGrade::Reproducible,
        "",
        None,
        vec![inner.id.clone()],
        vec![],
    );
    let middle = catalog.admit_evidence(middle, current).unwrap();
    let outer = claim(
        &ids,
        EvidenceGrade::Release,
        "",
        None,
        vec![middle.id.clone()],
        vec![],
    );
    let outer = catalog.admit_evidence(outer, current).unwrap();
    let inner_checker = ids[&EvidenceGrade::Bootstrap].clone();
    let revoke_inner = |id: &CheckerReceiptId| {
        if id == &inner_checker {
            RevocationStatus::Revoked
        } else {
            RevocationStatus::Current
        }
    };
    assert_eq!(
        catalog.decide_evidence(&outer.key, EvidenceGrade::Release, &revoke_inner),
        EvidenceDecision::Refuse {
            reason: "revoked-support",
        }
    );
    let late = claim(
        &ids,
        EvidenceGrade::Release,
        "late",
        None,
        vec![middle.id],
        vec![],
    );
    assert!(refused(
        catalog.admit_evidence(late, &revoke_inner),
        "revoked-support"
    ));
}

#[test]
fn support_sets_do_not_share_a_key() {
    let (mut catalog, ids) = graded_catalog();
    let support = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let bare = admit(&mut catalog, &ids, EvidenceGrade::Reproducible, "", None);
    let with_support = catalog
        .admit_evidence(
            claim(
                &ids,
                EvidenceGrade::Reproducible,
                "",
                None,
                vec![support.id.clone()],
                vec![],
            ),
            current,
        )
        .unwrap();
    assert_ne!(bare.key, with_support.key);
    let support_checker = ids[&EvidenceGrade::Bootstrap].clone();
    assert_eq!(
        catalog.decide_evidence(&bare.key, EvidenceGrade::Reproducible, |id| {
            if id == &support_checker {
                RevocationStatus::Revoked
            } else {
                RevocationStatus::Current
            }
        }),
        EvidenceDecision::Reuse { receipt: bare.id }
    );
    assert_eq!(
        catalog.decide_evidence(&with_support.key, EvidenceGrade::Reproducible, |id| {
            if id == &support_checker {
                RevocationStatus::Revoked
            } else {
                RevocationStatus::Current
            }
        },),
        EvidenceDecision::Refuse {
            reason: "revoked-support",
        }
    );
}

#[test]
fn distinct_projection_lists_do_not_share_a_receipt() {
    let (mut catalog, ids) = graded_catalog();
    let mut joined = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    joined.projections = vec!["a,b".into()];
    let mut split = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    split.projections = vec!["a".into(), "b".into()];
    assert!(refused(
        catalog.admit_evidence(joined, current),
        "unrelated-invocation"
    ));
    assert!(refused(
        catalog.admit_evidence(split, current),
        "unrelated-invocation"
    ));
}
