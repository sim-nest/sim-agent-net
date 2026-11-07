// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;

use sim_conformance_core::{
    CheckArgument, CheckScopeId, CheckTemplate, CheckerBinding, CheckerReceipt, CheckerReceiptId,
    EvidenceGrade, IdKind, PolicyId, RevocationStatus, SemanticId,
};

use super::{DiscoveredDefinition, EvidenceClaim, EvidenceDecision, discover_definitions};
use crate::{ProofCatalog, ProofLeaf, RetainedCheckerReceipt};

const SOURCE: &str = "\
sim-world-inputs-v1
root-package lantern
domain resolver/lantern . compiling Cargo.lock
domain resolver/lantern/vendor vendor enumerated vendor/Cargo.lock
member resolver/lantern path lantern 1.0.0 . aaaa1111 path-content closure
member resolver/lantern/vendor path widget 1.0.0 vendor/widgets bbbb2222 path-content outside
member resolver/lantern registry sim-kernel 0.4.0 registry+https://github.com/rust-lang/crates.io-index cccc3333 registry closure
generated resolver/lantern src/generated.rs 4 dddd4444
";

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
    let mut dependent = claim(
        &ids,
        EvidenceGrade::Reproducible,
        "",
        None,
        vec![support.id.clone()],
        vec![],
    );
    dependent.projections = vec!["projection/narrow".into()];
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
fn a_long_projection_keeps_a_distinct_key() {
    let (mut catalog, ids) = graded_catalog();
    let mut wide = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    wide.projections = vec!["p".repeat(20_000)];
    let wide = catalog.admit_evidence(wide, current).unwrap();
    let mut again = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    again.projections = vec!["p".repeat(20_000)];
    let again = catalog.admit_evidence(again, current).unwrap();
    assert_eq!(wide.key, again.key);
    assert_eq!(wide.id, again.id);
    let mut other = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    other.projections = vec!["q".repeat(20_000)];
    let other = catalog.admit_evidence(other, current).unwrap();
    assert_ne!(wide.key, other.key);
}

#[test]
fn revoked_indirect_support_blocks_admission_and_reuse() {
    let (mut catalog, ids) = graded_catalog();
    let inner = admit(&mut catalog, &ids, EvidenceGrade::Bootstrap, "", None);
    let mut middle = claim(
        &ids,
        EvidenceGrade::Reproducible,
        "",
        None,
        vec![inner.id.clone()],
        vec![],
    );
    middle.projections = vec!["projection/middle".into()];
    let middle = catalog.admit_evidence(middle, current).unwrap();
    let mut outer = claim(
        &ids,
        EvidenceGrade::Release,
        "",
        None,
        vec![middle.id.clone()],
        vec![],
    );
    outer.projections = vec!["projection/outer".into()];
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
    let mut late = claim(
        &ids,
        EvidenceGrade::Release,
        "late",
        None,
        vec![middle.id],
        vec![],
    );
    late.projections = vec!["projection/late".into()];
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
fn distinct_projection_lists_keep_distinct_keys() {
    let (mut catalog, ids) = graded_catalog();
    let mut joined = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    joined.projections = vec!["a,b".into()];
    let mut split = claim(&ids, EvidenceGrade::Bootstrap, "", None, vec![], vec![]);
    split.projections = vec!["a".into(), "b".into()];
    let joined = catalog.admit_evidence(joined, current).unwrap();
    let split = catalog.admit_evidence(split, current).unwrap();
    assert_ne!(joined.key, split.key);
}

fn row<'a>(found: &'a [DiscoveredDefinition], class: &str) -> &'a DiscoveredDefinition {
    found.iter().find(|item| item.class == class).unwrap()
}

fn refused(result: Result<super::AdmittedEvidence, crate::ProofError>, reason: &str) -> bool {
    matches!(result, Err(crate::ProofError::Refused(got)) if got == reason)
}

fn current(_: &CheckerReceiptId) -> RevocationStatus {
    RevocationStatus::Current
}

fn admit(
    catalog: &mut ProofCatalog,
    ids: &BTreeSetMap,
    grade: EvidenceGrade,
    telemetry: &str,
    requested: Option<EvidenceGrade>,
) -> super::AdmittedEvidence {
    catalog
        .admit_evidence(
            claim(ids, grade, telemetry, requested, vec![], vec![]),
            current,
        )
        .unwrap()
}

fn claim(
    ids: &BTreeSetMap,
    grade: EvidenceGrade,
    telemetry: &str,
    requested: Option<EvidenceGrade>,
    supports: Vec<super::EvidenceReceiptId>,
    extra_policies: Vec<PolicyId>,
) -> EvidenceClaim {
    let found = discover_definitions(SOURCE).unwrap();
    let workspace = row(&found, "workspace");
    let generated = row(&found, "generate");
    let mut policies = vec![sid("policy/catalog")];
    policies.extend(extra_policies);
    EvidenceClaim {
        proof: "anchor".into(),
        checker_receipt: ids[&grade].clone(),
        projections: vec!["projection/body".into()],
        environment: "environment/model".into(),
        source_text: SOURCE.into(),
        members: vec![workspace.semantic.clone(), generated.semantic.clone()],
        domains: vec![workspace.domain.clone()],
        policies,
        supports,
        telemetry: telemetry.into(),
        requested_grade: requested,
    }
}

type BTreeSetMap = std::collections::BTreeMap<EvidenceGrade, CheckerReceiptId>;

fn graded_catalog() -> (ProofCatalog, BTreeSetMap) {
    let binding = checker_binding();
    let invocation = binding
        .instantiate(
            sid("code/catalog"),
            sid("pack/catalog"),
            sid("subject/lantern"),
            sid("scope/evidence"),
            sid("closure/lantern"),
        )
        .unwrap();
    let mut catalog = ProofCatalog::new([ProofLeaf::ArtifactEquality {
        name: "anchor".into(),
        left: b"same".to_vec(),
        right: b"same".to_vec(),
    }])
    .unwrap();
    let mut ids = BTreeSetMap::new();
    for (grade, provenance, result) in [
        (
            EvidenceGrade::Bootstrap,
            "provenance/bootstrap",
            "result/bootstrap",
        ),
        (
            EvidenceGrade::Reproducible,
            "provenance/independent",
            "result/checked",
        ),
        (
            EvidenceGrade::Release,
            "provenance/published",
            "result/published",
        ),
    ] {
        let receipt = CheckerReceipt::passing(
            &invocation,
            sid(result),
            grade,
            sid(provenance),
            sid("policy/catalog"),
            sid("support/checker"),
            RevocationStatus::Current,
        )
        .unwrap();
        ids.insert(grade, receipt.id().clone());
        catalog = catalog
            .with_checker_receipt(
                RetainedCheckerReceipt::new(binding.clone(), invocation.clone(), receipt),
                RevocationStatus::Current,
            )
            .unwrap();
    }
    (catalog, ids)
}

fn sid<K: IdKind>(value: &str) -> SemanticId<K> {
    SemanticId::from_text(value).unwrap()
}

fn checker_binding() -> CheckerBinding {
    CheckerBinding::new(
        "checker/c-evidence".into(),
        sid("owner/test"),
        "packs::evidence::check".into(),
        vec![sid("pack/catalog")],
        sid("shape/check-receipt"),
        sid("revocation/test"),
        sid("command/validate"),
        sid("command/docs"),
        BTreeSet::<CheckScopeId>::from([sid("scope/evidence")]),
        CheckTemplate::new(
            "check".into(),
            vec![
                CheckArgument::BindingSlot,
                CheckArgument::SubjectSlot,
                CheckArgument::ScopeSlot,
            ],
            sid("cwd/source"),
            sid("environment/sealed"),
            sid("shape/check-result"),
        )
        .unwrap(),
    )
    .unwrap()
}
