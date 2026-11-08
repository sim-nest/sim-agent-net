// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Semantic evidence keys stored on the one proof catalog.
//!
//! A key names the checked subject and scope, the proof definition, proof
//! code, checker, consumed projections, environment, admitted source
//! sub-universe, and consumed policy. The retained checker invocation must
//! have checked that same claim. Grade and provenance live on the receipt.
//! Telemetry is not part of the key. The grade is the checker receipt's grade.

use sha2::{Digest, Sha256};
use sim_conformance_core::{
    CheckScopeId, CheckedSubjectId, CheckerReceiptId, EvidenceGrade, IdKind, PolicyId,
    RevocationStatus, SemanticId,
};
use sim_kernel::ContentId;

use super::{ProofCatalog, ProofError};

/// Semantic identity of one evidence key.
pub struct EvidenceKeyKind;
impl IdKind for EvidenceKeyKind {
    const DOMAIN: &'static str = "evidence/key-v1";
}
/// Identity of the fields that stay stable across grades.
pub type EvidenceKeyId = SemanticId<EvidenceKeyKind>;

/// Semantic identity of one immutable evidence receipt.
pub struct EvidenceReceiptKind;
impl IdKind for EvidenceReceiptKind {
    const DOMAIN: &'static str = "evidence/receipt-v1";
}
/// Identity of one grade-specific receipt.
pub type EvidenceReceiptId = SemanticId<EvidenceReceiptKind>;

/// One source definition read from sealed input text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredDefinition {
    /// `generate`, `standalone`, `workspace`, or `package`.
    pub class: &'static str,
    /// Resolver domain that contains the definition.
    pub domain: String,
    /// Package, path, or member name.
    pub name: String,
    /// Semantic identity. This is not the byte checksum.
    pub semantic: String,
    /// Byte checksum when the sealed text has one.
    pub storage: String,
}

/// Claim attached to a checker receipt already in this catalog.
pub struct EvidenceClaim {
    /// Catalog leaf this evidence explains.
    pub proof: String,
    /// Checked subject. It must be the invocation's subject.
    pub subject: CheckedSubjectId,
    /// Checked scope. It must be the invocation's scope.
    pub scope: CheckScopeId,
    /// Checker receipt whose result is being attached.
    pub checker_receipt: CheckerReceiptId,
    /// Projection digests the proof consumes.
    pub projections: Vec<String>,
    /// Environment identity the proof reads.
    pub environment: String,
    /// Sealed source text in the world input grammar.
    pub source_text: String,
    /// Semantic definition names that form the sub-universe.
    pub members: Vec<String>,
    /// Domains the proof declares. Members outside them are refused.
    pub domains: Vec<String>,
    /// Policies the proof reads. This must be exactly the checker policy.
    pub policies: Vec<PolicyId>,
    /// Evidence receipts whose support is required.
    pub supports: Vec<EvidenceReceiptId>,
    /// Caller note. It is stored nowhere in the key.
    pub telemetry: String,
    /// Grade the caller asked to record. It must match the checker receipt.
    pub requested_grade: Option<EvidenceGrade>,
}

/// Receipt admitted onto the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEvidence {
    /// Immutable receipt identity.
    pub id: EvidenceReceiptId,
    /// Grade-stable key.
    pub key: EvidenceKeyId,
    /// Grade copied from the checker receipt.
    pub grade: EvidenceGrade,
}

/// Reuse decision for one key and required grade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceDecision {
    /// A current receipt meets the required grade.
    Reuse {
        /// Receipt that meets the claim.
        receipt: EvidenceReceiptId,
    },
    /// A current receipt exists and is weaker than required.
    Recheck {
        /// Why the current receipts are not enough.
        reason: &'static str,
    },
    /// Reuse is refused.
    Refuse {
        /// Stable refusal name.
        reason: &'static str,
    },
}

#[derive(Clone, Debug)]
pub(super) struct EvidenceRecord {
    key: EvidenceKeyId,
    grade: EvidenceGrade,
    supports: Vec<EvidenceReceiptId>,
    checker_receipt: CheckerReceiptId,
}

#[path = "evidence_discover.rs"]
mod discover;
pub use discover::discover_definitions;

impl ProofCatalog {
    /// Attaches one immutable receipt to this catalog.
    pub fn admit_evidence<F>(
        &mut self,
        claim: EvidenceClaim,
        mut revocation: F,
    ) -> Result<AdmittedEvidence, ProofError>
    where
        F: FnMut(&CheckerReceiptId) -> RevocationStatus,
    {
        let (definition, code) = {
            let leaf = self.leaf(&claim.proof)?;
            (bind::proof_definition(leaf), bind::proof_code(leaf))
        };
        let retained = self.require_checker(&claim.checker_receipt, &mut revocation)?;
        let grade = retained.receipt().grade();
        if claim
            .requested_grade
            .is_some_and(|requested| requested != grade)
        {
            return Err(ProofError::Refused("grade-conversion"));
        }
        if claim.policies.len() != 1 || &claim.policies[0] != retained.receipt().policy() {
            return Err(ProofError::Refused("unrelated-policy"));
        }
        let _telemetry = claim.telemetry.as_str();
        let definitions = discover_definitions(&claim.source_text)?;
        let selected = select_members(&definitions, &claim)?;
        bind::require_invocation(retained.invocation(), &claim, &definition, &code, &selected)?;
        self.require_support_graph(&claim.supports, &mut revocation)?;
        let universe = bind::universe_text(&selected, true);
        let checker = bind::checker_preimage(
            retained.binding().checker(),
            retained.invocation().checker_code().content_id(),
        );
        let key = semantic(&canon(&[
            ("subject", &hex(claim.subject.content_id())),
            ("scope", &hex(claim.scope.content_id())),
            ("proof", &digest(&definition)),
            ("proof-code", &digest(&code)),
            ("checker", &digest(&checker)),
            ("inputs", &digest(&sorted_join(&claim.projections))),
            ("environment", &digest(claim.environment.as_str())),
            ("universe", &digest(&universe)),
            ("supports", &digest(&sorted_ids(&claim.supports))),
            ("policy", &hex(retained.receipt().policy().content_id())),
        ]))?;
        let id = semantic(&canon(&[
            ("key", &hex(key.content_id())),
            ("grade", grade_label(grade)),
            ("result", &hex(retained.receipt().result().content_id())),
            (
                "provenance",
                &hex(retained.receipt().provenance().content_id()),
            ),
            ("supports", &digest(&sorted_ids(&claim.supports))),
        ]))?;
        let record = EvidenceRecord {
            key: key.clone(),
            grade,
            supports: claim.supports,
            checker_receipt: claim.checker_receipt,
        };
        if let Some(existing) = self.evidence.get(&id) {
            if existing.key != key {
                return Err(ProofError::Invalid("evidence receipt changed".into()));
            }
            return Ok(AdmittedEvidence { id, key, grade });
        }
        self.evidence.insert(id.clone(), record);
        Ok(AdmittedEvidence { id, key, grade })
    }

    /// Decides reuse from current revocation and the required grade.
    pub fn decide_evidence<F>(
        &self,
        key: &EvidenceKeyId,
        required: EvidenceGrade,
        mut revocation: F,
    ) -> EvidenceDecision
    where
        F: FnMut(&CheckerReceiptId) -> RevocationStatus,
    {
        let mut eligible = Vec::new();
        let mut revoked = false;
        let mut unknown = false;
        let mut weak = false;
        let mut saw = false;
        for (id, record) in &self.evidence {
            if &record.key != key {
                continue;
            }
            saw = true;
            match self.gate(record, required, &mut revocation) {
                Gate::Open => eligible.push((record.grade, id.clone())),
                Gate::Revoked => revoked = true,
                Gate::Unknown => unknown = true,
                Gate::Weak => weak = true,
                Gate::Missing => {
                    return EvidenceDecision::Refuse {
                        reason: "missing-object",
                    };
                }
            }
        }
        if !saw {
            return EvidenceDecision::Refuse {
                reason: "missing-object",
            };
        }
        if !eligible.is_empty() {
            eligible.sort();
            return EvidenceDecision::Reuse {
                receipt: eligible[0].1.clone(),
            };
        }
        if revoked {
            EvidenceDecision::Refuse {
                reason: "revoked-support",
            }
        } else if unknown {
            EvidenceDecision::Refuse {
                reason: "unknown-revocation",
            }
        } else if weak {
            EvidenceDecision::Recheck { reason: "grade" }
        } else {
            EvidenceDecision::Refuse {
                reason: "missing-object",
            }
        }
    }

    fn require_checker<F>(
        &self,
        id: &CheckerReceiptId,
        revocation: &mut F,
    ) -> Result<&super::RetainedCheckerReceipt, ProofError>
    where
        F: FnMut(&CheckerReceiptId) -> RevocationStatus,
    {
        let status = revocation(id);
        if status == RevocationStatus::Revoked {
            return Err(ProofError::Refused("revoked-support"));
        }
        if status == RevocationStatus::Unknown {
            return Err(ProofError::Refused("unknown-revocation"));
        }
        let retained = self.retained_checker(id)?;
        retained.verify(status)?;
        Ok(retained)
    }

    fn require_support_graph<F>(
        &self,
        supports: &[EvidenceReceiptId],
        revocation: &mut F,
    ) -> Result<(), ProofError>
    where
        F: FnMut(&CheckerReceiptId) -> RevocationStatus,
    {
        for checker in self.support_checkers(supports)? {
            self.require_checker(&checker, revocation)?;
        }
        Ok(())
    }

    fn gate<F>(&self, record: &EvidenceRecord, required: EvidenceGrade, revocation: &mut F) -> Gate
    where
        F: FnMut(&CheckerReceiptId) -> RevocationStatus,
    {
        let meets = record.grade >= required;
        let mut ids = match self.support_checkers(&record.supports) {
            Ok(found) => found,
            Err(_) => return Gate::Missing,
        };
        ids.push(record.checker_receipt.clone());
        let mut unknown = false;
        for id in ids {
            match revocation(&id) {
                RevocationStatus::Revoked if meets => return Gate::Revoked,
                RevocationStatus::Revoked => return Gate::Weak,
                RevocationStatus::Unknown => unknown = true,
                RevocationStatus::Current => {}
            }
        }
        if unknown {
            if meets { Gate::Unknown } else { Gate::Weak }
        } else if meets {
            Gate::Open
        } else {
            Gate::Weak
        }
    }

    fn support_checkers(
        &self,
        supports: &[EvidenceReceiptId],
    ) -> Result<Vec<CheckerReceiptId>, ProofError> {
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = supports.to_vec();
        let mut checkers = Vec::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let record = self
                .evidence
                .get(&id)
                .ok_or(ProofError::Refused("missing-object"))?;
            checkers.push(record.checker_receipt.clone());
            stack.extend(record.supports.iter().cloned());
        }
        Ok(checkers)
    }
}

enum Gate {
    Open,
    Revoked,
    Unknown,
    Weak,
    Missing,
}

fn select_members<'a>(
    definitions: &'a [DiscoveredDefinition],
    claim: &EvidenceClaim,
) -> Result<Vec<&'a DiscoveredDefinition>, ProofError> {
    if claim.members.is_empty() || claim.domains.is_empty() || claim.environment.is_empty() {
        return Err(ProofError::Refused("missing-object"));
    }
    let known: std::collections::BTreeSet<&str> =
        definitions.iter().map(|row| row.domain.as_str()).collect();
    if claim
        .domains
        .iter()
        .any(|domain| !known.contains(domain.as_str()))
    {
        return Err(ProofError::Refused("wrong-domain"));
    }
    let mut selected = Vec::new();
    for name in &claim.members {
        if crosses(name, definitions) {
            return Err(ProofError::Refused("storage-id-crossing"));
        }
        let Some(found) = definitions.iter().find(|row| &row.semantic == name) else {
            return Err(ProofError::Refused("missing-object"));
        };
        if !claim.domains.iter().any(|domain| domain == &found.domain) {
            return Err(ProofError::Refused("wrong-domain"));
        }
        selected.push(found);
    }
    for projection in &claim.projections {
        if stored(projection) && definitions.iter().any(|row| row.storage == *projection) {
            return Err(ProofError::Refused("storage-id-crossing"));
        }
    }
    selected.sort_by(|left, right| left.semantic.cmp(&right.semantic));
    selected.dedup_by(|left, right| left.semantic == right.semantic);
    Ok(selected)
}

fn crosses(name: &str, definitions: &[DiscoveredDefinition]) -> bool {
    stored(name)
        && definitions
            .iter()
            .any(|row| row.storage == name && row.semantic != name)
}

fn stored(value: &str) -> bool {
    !value.is_empty() && value != "-"
}

fn sorted_join(values: &[String]) -> String {
    let mut values = values.to_vec();
    values.sort();
    length_prefixed(&values)
}

fn sorted_ids(values: &[EvidenceReceiptId]) -> String {
    let mut values = values
        .iter()
        .map(|id| hex(id.content_id()))
        .collect::<Vec<_>>();
    values.sort();
    length_prefixed(&values)
}

fn length_prefixed(values: &[String]) -> String {
    let mut out = String::new();
    for value in values {
        out.push_str(&value.len().to_string());
        out.push(':');
        out.push_str(value);
        out.push(';');
    }
    out
}

fn canon(fields: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, value) in fields {
        out.push_str(name);
        out.push('=');
        out.push_str(&value.len().to_string());
        out.push(':');
        out.push_str(value);
        out.push(';');
    }
    out
}

fn hex(id: &ContentId) -> String {
    hex_bytes(&id.bytes)
}

fn digest(text: &str) -> String {
    hex_bytes(Sha256::digest(text.as_bytes()).as_slice())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn semantic<K: IdKind>(text: &str) -> Result<SemanticId<K>, ProofError> {
    SemanticId::from_text(text).map_err(|error| ProofError::Invalid(error.to_string()))
}

fn grade_label(grade: EvidenceGrade) -> &'static str {
    match grade {
        EvidenceGrade::Bootstrap => "bootstrap",
        EvidenceGrade::Reproducible => "reproducible",
        EvidenceGrade::Release => "release",
    }
}

#[path = "evidence_bind.rs"]
mod bind;

#[cfg(test)]
#[path = "evidence_fixture.rs"]
mod evidence_fixture;

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod evidence_tests;

#[cfg(test)]
#[path = "evidence_repair_tests.rs"]
mod evidence_repair_tests;

#[cfg(test)]
#[path = "evidence_discover_tests.rs"]
mod evidence_discover_tests;
