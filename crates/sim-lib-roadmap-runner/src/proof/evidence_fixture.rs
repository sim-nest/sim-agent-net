// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use sim_conformance_core::{
    CheckArgument, CheckInputClosureId, CheckScopeId, CheckTemplate, CheckerBinding,
    CheckerReceipt, CheckerReceiptId, EvidenceGrade, IdKind, PolicyId, RevocationStatus,
    SemanticId,
};

use super::{DiscoveredDefinition, EvidenceClaim, discover_definitions};
use crate::{ProofCatalog, ProofLeaf, RetainedCheckerReceipt};

pub(crate) const SOURCE: &str = "\
sim-world-inputs-v1
root-package lantern
domain resolver/lantern . compiling Cargo.lock
domain resolver/lantern/vendor vendor enumerated vendor/Cargo.lock
member resolver/lantern path lantern 1.0.0 . aaaa1111 path-content closure
member resolver/lantern/vendor path widget 1.0.0 vendor/widgets bbbb2222 path-content outside
member resolver/lantern registry sim-kernel 0.4.0 registry+https://github.com/rust-lang/crates.io-index cccc3333 registry closure
generated resolver/lantern src/generated.rs 4 dddd4444
";

pub(crate) type GradeIds = BTreeMap<EvidenceGrade, CheckerReceiptId>;

pub(crate) fn anchor_leaf() -> ProofLeaf {
    ProofLeaf::ArtifactEquality {
        name: "anchor".into(),
        left: b"same".to_vec(),
        right: b"same".to_vec(),
    }
}

pub(crate) fn row<'a>(found: &'a [DiscoveredDefinition], class: &str) -> &'a DiscoveredDefinition {
    found.iter().find(|item| item.class == class).unwrap()
}

pub(crate) fn refused(
    result: Result<super::AdmittedEvidence, crate::ProofError>,
    reason: &str,
) -> bool {
    matches!(result, Err(crate::ProofError::Refused(got)) if got == reason)
}

pub(crate) fn current(_: &CheckerReceiptId) -> RevocationStatus {
    RevocationStatus::Current
}

pub(crate) fn admit(
    catalog: &mut ProofCatalog,
    ids: &GradeIds,
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

pub(crate) fn claim(
    ids: &GradeIds,
    grade: EvidenceGrade,
    telemetry: &str,
    requested: Option<EvidenceGrade>,
    supports: Vec<super::EvidenceReceiptId>,
    extra_policies: Vec<PolicyId>,
) -> EvidenceClaim {
    let mut built = template(
        SOURCE,
        &["projection/body".into()],
        "subject/lantern",
        "scope/evidence",
    );
    built.checker_receipt = ids[&grade].clone();
    built.telemetry = telemetry.into();
    built.requested_grade = requested;
    built.supports = supports;
    built.policies.extend(extra_policies);
    built
}

pub(crate) fn graded_catalog() -> (ProofCatalog, GradeIds) {
    catalog_for(
        anchor_leaf(),
        &["projection/body".into()],
        SOURCE,
        "subject/lantern",
        true,
    )
}

pub(crate) fn catalog_for(
    leaf: ProofLeaf,
    projections: &[String],
    source: &str,
    subject: &str,
    bind_storage: bool,
) -> (ProofCatalog, GradeIds) {
    let closure = closure_for(
        &leaf,
        projections,
        source,
        subject,
        "scope/evidence",
        bind_storage,
    );
    catalog_with_closure(leaf, closure, subject)
}

pub(crate) fn closure_for(
    leaf: &ProofLeaf,
    projections: &[String],
    source: &str,
    subject: &str,
    scope: &str,
    bind_storage: bool,
) -> CheckInputClosureId {
    let built = template(source, projections, subject, scope);
    let found = discover_definitions(source).unwrap();
    let selected = super::select_members(&found, &built).unwrap();
    super::bind::attachment_closure(
        &built,
        &super::bind::proof_definition(leaf),
        &super::bind::proof_code(leaf),
        &selected,
        bind_storage,
    )
    .unwrap()
}

pub(crate) fn catalog_with_closure(
    leaf: ProofLeaf,
    closure: CheckInputClosureId,
    subject: &str,
) -> (ProofCatalog, GradeIds) {
    let binding = checker_binding();
    let invocation = binding
        .instantiate(
            sid("code/catalog"),
            sid("pack/catalog"),
            sid(subject),
            sid("scope/evidence"),
            closure,
        )
        .unwrap();
    let mut catalog = ProofCatalog::new([leaf]).unwrap();
    let mut ids = GradeIds::new();
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

pub(crate) fn install_release(
    catalog: ProofCatalog,
    subject: &str,
    scope: &str,
    closure: CheckInputClosureId,
) -> (ProofCatalog, CheckerReceiptId) {
    let binding = checker_binding();
    let invocation = binding
        .instantiate(
            sid("code/catalog"),
            sid("pack/catalog"),
            sid(subject),
            sid(scope),
            closure,
        )
        .unwrap();
    let receipt = CheckerReceipt::passing(
        &invocation,
        sid("result/other-release"),
        EvidenceGrade::Release,
        sid("provenance/other-release"),
        sid("policy/catalog"),
        sid("support/checker"),
        RevocationStatus::Current,
    )
    .unwrap();
    let id = receipt.id().clone();
    let catalog = catalog
        .with_checker_receipt(
            RetainedCheckerReceipt::new(binding, invocation, receipt),
            RevocationStatus::Current,
        )
        .unwrap();
    (catalog, id)
}

fn template(source: &str, projections: &[String], subject: &str, scope: &str) -> EvidenceClaim {
    let found = discover_definitions(source).unwrap();
    let workspace = row(&found, "workspace");
    let generated = row(&found, "generate");
    EvidenceClaim {
        proof: "anchor".into(),
        subject: sid(subject),
        scope: sid(scope),
        checker_receipt: sid("receipt/unused"),
        projections: projections.to_vec(),
        environment: "environment/model".into(),
        source_text: source.into(),
        members: vec![workspace.semantic.clone(), generated.semantic.clone()],
        domains: vec![workspace.domain.clone()],
        policies: vec![sid("policy/catalog")],
        supports: vec![],
        telemetry: String::new(),
        requested_grade: None,
    }
}

pub(crate) fn sid<K: IdKind>(value: &str) -> SemanticId<K> {
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
        BTreeSet::<CheckScopeId>::from([sid("scope/evidence"), sid("scope/other")]),
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
