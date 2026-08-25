use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_kernel::Symbol;
use sim_roadmap_core::PromiseId;
use sim_roadmap_exec_core::{PromiseDischarge, UnresolvedProof};

use crate::{ProofDisposition, TypedProofReceipt};

/// Every identity that makes a proof receipt eligible for one grounded execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofAuthority {
    pub plan: ContentId,
    pub deck: ContentId,
    pub mutation: ContentId,
    pub launcher: String,
    pub policy: ContentId,
    pub proof_definition: ContentId,
}

#[derive(Clone, Debug)]
pub struct GroundedPromise {
    pub id: PromiseId,
    pub admitted_proofs: BTreeMap<String, ContentId>,
    /// An inconclusive proof may use only this proof, named before execution.
    pub inconclusive_fallback: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CorrelatedProof {
    pub authority: ProofAuthority,
    pub receipt: TypedProofReceipt,
    pub evidence: ContentId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Proven,
    Refuted,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromiseDecision {
    pub promise: PromiseId,
    pub disposition: Disposition,
    pub evidence: Option<ContentId>,
    pub proof: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptanceFailure {
    ForeignAuthority,
    UnadmittedProof(String),
    ChangedProofDefinition(String),
    MissingFallback(PromiseId),
    ExhaustedFallbackBudget,
    Refuted(PromiseId),
    Inconclusive(PromiseId),
    MissingPromise(PromiseId),
    ParentCoverageHole(PromiseId),
}

/// Judge receipts semantically. Exit zero alone is deliberately not consulted here.
pub fn decide_promise(
    promise: &GroundedPromise,
    expected: &ProofAuthority,
    primary: &CorrelatedProof,
    fallback: Option<&CorrelatedProof>,
    fallback_budget: &mut usize,
) -> Result<PromiseDecision, AcceptanceFailure> {
    validate_receipt(promise, expected, primary)?;
    match primary.receipt.disposition {
        ProofDisposition::Passed => Ok(decision(promise, primary, Disposition::Proven)),
        ProofDisposition::Failed => Ok(decision(promise, primary, Disposition::Refuted)),
        ProofDisposition::Ambiguous => {
            let fallback_name = promise
                .inconclusive_fallback
                .as_deref()
                .ok_or_else(|| AcceptanceFailure::MissingFallback(promise.id.clone()))?;
            if *fallback_budget == 0 {
                return Err(AcceptanceFailure::ExhaustedFallbackBudget);
            }
            let fallback = fallback
                .filter(|proof| proof.receipt.proof == fallback_name)
                .ok_or_else(|| AcceptanceFailure::MissingFallback(promise.id.clone()))?;
            validate_receipt(promise, expected, fallback)?;
            *fallback_budget -= 1;
            Ok(decision(
                promise,
                fallback,
                match fallback.receipt.disposition {
                    ProofDisposition::Passed => Disposition::Proven,
                    ProofDisposition::Failed => Disposition::Refuted,
                    ProofDisposition::Ambiguous => Disposition::Inconclusive,
                },
            ))
        }
    }
}

fn validate_receipt(
    promise: &GroundedPromise,
    expected: &ProofAuthority,
    proof: &CorrelatedProof,
) -> Result<(), AcceptanceFailure> {
    if &proof.authority != expected
        || proof
            .receipt
            .launcher_identity
            .as_deref()
            .is_some_and(|v| v != expected.launcher)
    {
        return Err(AcceptanceFailure::ForeignAuthority);
    }
    let definition = promise
        .admitted_proofs
        .get(&proof.receipt.proof)
        .ok_or_else(|| AcceptanceFailure::UnadmittedProof(proof.receipt.proof.clone()))?;
    if definition != &expected.proof_definition {
        return Err(AcceptanceFailure::ChangedProofDefinition(
            proof.receipt.proof.clone(),
        ));
    }
    Ok(())
}

fn decision(
    promise: &GroundedPromise,
    proof: &CorrelatedProof,
    disposition: Disposition,
) -> PromiseDecision {
    PromiseDecision {
        promise: promise.id.clone(),
        disposition,
        evidence: (disposition == Disposition::Proven).then(|| proof.evidence.clone()),
        proof: proof.receipt.proof.clone(),
    }
}

/// A parent promise names the child promises whose evidence contributes to it.
#[derive(Clone, Debug, Default)]
pub struct ParentAcceptance {
    pub required: BTreeMap<PromiseId, BTreeSet<PromiseId>>,
}

pub fn accept_all(
    promises: &[GroundedPromise],
    decisions: &[PromiseDecision],
    parent: &ParentAcceptance,
) -> Result<Vec<PromiseDischarge>, AcceptanceFailure> {
    let by_id: BTreeMap<_, _> = decisions.iter().map(|d| (&d.promise, d)).collect();
    for promise in promises {
        let decision = by_id
            .get(&promise.id)
            .ok_or_else(|| AcceptanceFailure::MissingPromise(promise.id.clone()))?;
        match decision.disposition {
            Disposition::Proven => {}
            Disposition::Refuted => return Err(AcceptanceFailure::Refuted(promise.id.clone())),
            Disposition::Inconclusive => {
                return Err(AcceptanceFailure::Inconclusive(promise.id.clone()));
            }
        }
    }
    for (parent_id, contributors) in &parent.required {
        if contributors.is_empty()
            || contributors.iter().any(|id| {
                by_id
                    .get(id)
                    .is_none_or(|decision| decision.disposition != Disposition::Proven)
            })
        {
            return Err(AcceptanceFailure::ParentCoverageHole(parent_id.clone()));
        }
    }
    Ok(decisions
        .iter()
        .map(|decision| PromiseDischarge {
            promise: decision.promise.clone(),
            status: Symbol::new("proven"),
            evidence: decision.evidence.clone(),
        })
        .collect())
}

pub fn unresolved(decision: &PromiseDecision) -> Option<UnresolvedProof> {
    (decision.disposition != Disposition::Proven).then(|| UnresolvedProof {
        proof: Symbol::new(decision.proof.as_str()),
        mandatory: true,
        reason: Symbol::new(match decision.disposition {
            Disposition::Refuted => "refuted",
            Disposition::Inconclusive => "inconclusive",
            Disposition::Proven => unreachable!(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProofDisposition;
    use sim_kernel::Symbol;

    fn content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }
    fn authority() -> ProofAuthority {
        ProofAuthority {
            plan: content(1),
            deck: content(2),
            mutation: content(3),
            launcher: "networkless-v1".into(),
            policy: content(4),
            proof_definition: content(5),
        }
    }
    fn promise(fallback: Option<&str>) -> GroundedPromise {
        GroundedPromise {
            id: PromiseId::new("public-signature").unwrap(),
            admitted_proofs: [
                ("exact-source".into(), content(5)),
                ("fallback".into(), content(5)),
            ]
            .into(),
            inconclusive_fallback: fallback.map(str::to_owned),
        }
    }
    fn proof(name: &str, disposition: ProofDisposition) -> CorrelatedProof {
        CorrelatedProof {
            authority: authority(),
            receipt: TypedProofReceipt {
                proof: name.into(),
                effect_id: None,
                disposition,
                exit_code: Some(0),
                timeout: false,
                signal: None,
                truncated: false,
                launcher_identity: Some("networkless-v1".into()),
                sandbox_identity: Some("sandbox".into()),
                stdout_object: None,
                stderr_object: None,
                observed_at: "logical:1".into(),
                semantic_detail: "typed predicate".into(),
            },
            evidence: content(9),
        }
    }

    #[test]
    fn green_irrelevant_command_cannot_discharge_false_signature() {
        let mut irrelevant = proof("cargo-green", ProofDisposition::Passed);
        irrelevant.receipt.exit_code = Some(0);
        assert_eq!(
            decide_promise(&promise(None), &authority(), &irrelevant, None, &mut 0),
            Err(AcceptanceFailure::UnadmittedProof("cargo-green".into()))
        );
        let refuted = proof("exact-source", ProofDisposition::Failed);
        let decision =
            decide_promise(&promise(None), &authority(), &refuted, None, &mut 0).unwrap();
        assert_eq!(decision.disposition, Disposition::Refuted);
        assert_eq!(
            accept_all(&[promise(None)], &[decision], &ParentAcceptance::default()),
            Err(AcceptanceFailure::Refuted(
                PromiseId::new("public-signature").unwrap()
            ))
        );
    }

    #[test]
    fn exact_source_proof_is_stable_and_rejects_cross_run_receipts() {
        let exact = proof("exact-source", ProofDisposition::Passed);
        let first = decide_promise(&promise(None), &authority(), &exact, None, &mut 0).unwrap();
        let replay = decide_promise(&promise(None), &authority(), &exact, None, &mut 0).unwrap();
        assert_eq!(first, replay);
        let mut foreign = exact;
        foreign.authority.mutation = content(8);
        assert_eq!(
            decide_promise(&promise(None), &authority(), &foreign, None, &mut 0),
            Err(AcceptanceFailure::ForeignAuthority)
        );
    }

    #[test]
    fn inconclusive_requires_predeclared_budgeted_fallback() {
        let ambiguous = proof("exact-source", ProofDisposition::Ambiguous);
        let fallback = proof("fallback", ProofDisposition::Passed);
        assert!(matches!(
            decide_promise(
                &promise(None),
                &authority(),
                &ambiguous,
                Some(&fallback),
                &mut 1
            ),
            Err(AcceptanceFailure::MissingFallback(_))
        ));
        assert_eq!(
            decide_promise(
                &promise(Some("fallback")),
                &authority(),
                &ambiguous,
                Some(&fallback),
                &mut 0
            ),
            Err(AcceptanceFailure::ExhaustedFallbackBudget)
        );
        let mut budget = 1;
        let decision = decide_promise(
            &promise(Some("fallback")),
            &authority(),
            &ambiguous,
            Some(&fallback),
            &mut budget,
        )
        .unwrap();
        assert_eq!(decision.disposition, Disposition::Proven);
        assert_eq!(budget, 0);
    }

    #[test]
    fn completed_children_do_not_hide_parent_coverage_hole() {
        let decision = decide_promise(
            &promise(None),
            &authority(),
            &proof("exact-source", ProofDisposition::Passed),
            None,
            &mut 0,
        )
        .unwrap();
        let parent_id = PromiseId::new("parent-contract").unwrap();
        let parent = ParentAcceptance {
            required: [(
                parent_id.clone(),
                [PromiseId::new("uncovered-child").unwrap()].into(),
            )]
            .into(),
        };
        assert_eq!(
            accept_all(&[promise(None)], &[decision], &parent),
            Err(AcceptanceFailure::ParentCoverageHole(parent_id))
        );
    }
}
