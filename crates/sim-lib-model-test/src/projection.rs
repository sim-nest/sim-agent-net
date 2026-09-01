//! Model-specific, read-only projections of generic study decisions and selections.

use sim_kernel::{ContentId, Symbol};
use sim_lib_study::decision::{Selection, SubjectDecision, Verdict};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelVerdict {
    InsufficientEvidence,
    Unfit,
    TooExpensive,
    Usable,
    Preferred,
    Incomparable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CostReason {
    Ceiling { ceiling_minor: u64 },
    DominatedBy { candidate: ContentId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerdictFacts {
    pub required_independent_epochs: u32,
    pub failed_hard_gate_epochs: BTreeSet<ContentId>,
    pub price_revision: Option<ContentId>,
    pub cost_reconciled: bool,
    pub cost_reason: Option<CostReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDecision {
    /// The complete generic decision is retained, including all evidence and precedence reasons.
    pub generic: SubjectDecision,
    pub verdict: ModelVerdict,
    pub cost_reason: Option<CostReason>,
}

pub fn project_verdict(generic: SubjectDecision, facts: &VerdictFacts) -> ModelDecision {
    let (verdict, cost_reason) = match generic.verdict {
        Verdict::RejectedGate
            if facts.failed_hard_gate_epochs.len()
                >= usize::max(2, facts.required_independent_epochs as usize) =>
        {
            (ModelVerdict::Unfit, None)
        }
        Verdict::RejectedGate => (ModelVerdict::InsufficientEvidence, None),
        Verdict::OverBudget
            if facts.cost_reconciled
                && facts.price_revision.is_some()
                && facts.cost_reason.is_some() =>
        {
            (ModelVerdict::TooExpensive, facts.cost_reason.clone())
        }
        Verdict::OverBudget => (ModelVerdict::InsufficientEvidence, None),
        Verdict::Eligible | Verdict::Dominated => (ModelVerdict::Usable, None),
        Verdict::Preferred => (ModelVerdict::Preferred, None),
        Verdict::Incomparable => (ModelVerdict::Incomparable, None),
        Verdict::InsufficientEvidence => (ModelVerdict::InsufficientEvidence, None),
    };
    ModelDecision {
        generic,
        verdict,
        cost_reason,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioDisposition {
    Preferred,
    Specialist,
    Reserve,
    EconomicallyDominated,
    RetireCandidate,
    InsufficientEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleResult {
    pub role: Symbol,
    pub domain: Symbol,
    pub active: bool,
    pub uniquely_capable: bool,
    pub decision: ModelDecision,
}

pub fn portfolio_disposition(results: &[RoleResult]) -> PortfolioDisposition {
    let active = results.iter().filter(|r| r.active).collect::<Vec<_>>();
    if active.is_empty()
        || active
            .iter()
            .any(|r| r.decision.verdict == ModelVerdict::InsufficientEvidence)
    {
        return PortfolioDisposition::InsufficientEvidence;
    }
    if active.iter().any(|r| r.uniquely_capable) {
        return PortfolioDisposition::Specialist;
    }
    if active
        .iter()
        .all(|r| r.decision.verdict == ModelVerdict::Unfit)
    {
        return PortfolioDisposition::RetireCandidate;
    }
    if active
        .iter()
        .any(|r| r.decision.verdict == ModelVerdict::Preferred)
    {
        return PortfolioDisposition::Preferred;
    }
    if active
        .iter()
        .all(|r| r.decision.verdict == ModelVerdict::TooExpensive)
    {
        return PortfolioDisposition::EconomicallyDominated;
    }
    PortfolioDisposition::Reserve
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceInterval {
    pub resource: Symbol,
    pub lower: u64,
    pub upper: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateRoute {
    pub candidate_revision: ContentId,
    pub provider_seat: ContentId,
    pub route_semantics: Symbol,
    pub driver_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickFacts {
    pub complete: bool,
    pub bootstrap: bool,
    pub report_only: bool,
    pub private_approved: bool,
    pub unresolved: bool,
    pub quarantined: bool,
    pub subject_current: bool,
    pub price_current: bool,
    pub seats_current: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickRefusal {
    Incomplete,
    Bootstrap,
    ReportOnly,
    PrivateUnapproved,
    Unresolved,
    Quarantined,
    StaleSubject,
    StalePrice,
    StaleSeat,
}

impl PickRefusal {
    pub fn missing_fact(self) -> &'static str {
        match self {
            Self::Incomplete => "complete-evidence",
            Self::Bootstrap => "non-bootstrap-evidence",
            Self::ReportOnly => "selection-bearing-evidence",
            Self::PrivateUnapproved => "private-approval",
            Self::Unresolved => "resolved-evidence",
            Self::Quarantined => "non-quarantined-evidence",
            Self::StaleSubject => "current-subject",
            Self::StalePrice => "current-price",
            Self::StaleSeat => "current-provider-seat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPick {
    pub role: Symbol,
    pub candidates: Vec<CandidateRoute>,
    pub verdict: ModelVerdict,
    pub decisive_gates: Vec<Symbol>,
    pub resources: Vec<ResourceInterval>,
    pub epoch: ContentId,
    pub report_id: ContentId,
    pub expires_at_ms: u64,
    pub fallback_compatible: bool,
    /// The sole generic selection from which this record was projected.
    pub selection: Selection,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickJournal {
    entries: Vec<ModelPick>,
}
impl PickJournal {
    pub fn entries(&self) -> &[ModelPick] {
        &self.entries
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emit_pick(
    journal: &mut PickJournal,
    selection: Selection,
    role: Symbol,
    candidates: Vec<CandidateRoute>,
    verdict: ModelVerdict,
    decisive_gates: Vec<Symbol>,
    resources: Vec<ResourceInterval>,
    epoch: ContentId,
    expires_at_ms: u64,
    fallback_compatible: bool,
    facts: &PickFacts,
) -> Result<ModelPick, PickRefusal> {
    let refusal = if !facts.complete {
        Some(PickRefusal::Incomplete)
    } else if facts.bootstrap {
        Some(PickRefusal::Bootstrap)
    } else if facts.report_only {
        Some(PickRefusal::ReportOnly)
    } else if !facts.private_approved {
        Some(PickRefusal::PrivateUnapproved)
    } else if facts.unresolved {
        Some(PickRefusal::Unresolved)
    } else if facts.quarantined {
        Some(PickRefusal::Quarantined)
    } else if !facts.subject_current {
        Some(PickRefusal::StaleSubject)
    } else if !facts.price_current {
        Some(PickRefusal::StalePrice)
    } else if !facts.seats_current {
        Some(PickRefusal::StaleSeat)
    } else {
        None
    };
    if let Some(reason) = refusal {
        return Err(reason);
    }
    let pick = ModelPick {
        role,
        candidates,
        verdict,
        decisive_gates,
        resources,
        epoch,
        report_id: selection.report_root.clone(),
        expires_at_ms,
        fallback_compatible,
        selection,
    };
    journal.entries.push(pick.clone());
    Ok(pick)
}

impl ModelPick {
    pub fn rationale(&self) -> String {
        format!(
            "role={} verdict={:?} report={:?} expires-at-ms={} gates={}",
            self.role,
            self.verdict,
            self.report_id,
            self.expires_at_ms,
            self.decisive_gates
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
    pub fn driver_selection(&self) -> &str {
        self.candidates
            .first()
            .map(|c| c.driver_selection.as_str())
            .unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_study::decision::{StalenessInputs, SubjectDecision};
    fn id(n: u8) -> ContentId {
        ContentId::from_bytes(Symbol::new("sha256"), [n; 32])
    }
    fn decision(v: Verdict) -> SubjectDecision {
        SubjectDecision {
            subject: id(1),
            verdict: v,
            decisive_evidence: [id(2)].into(),
            decisive_inferences: [id(3)].into(),
            decisive_attributions: [id(4)].into(),
            reasons: vec![Symbol::new("generic-reason")],
        }
    }
    fn facts() -> VerdictFacts {
        VerdictFacts {
            required_independent_epochs: 2,
            failed_hard_gate_epochs: [id(5), id(6)].into(),
            price_revision: Some(id(7)),
            cost_reconciled: true,
            cost_reason: Some(CostReason::Ceiling { ceiling_minor: 10 }),
        }
    }
    #[test]
    fn every_verdict_and_precedence_preserves_generic_evidence() {
        let cases = [
            (
                Verdict::InsufficientEvidence,
                ModelVerdict::InsufficientEvidence,
            ),
            (Verdict::RejectedGate, ModelVerdict::Unfit),
            (Verdict::OverBudget, ModelVerdict::TooExpensive),
            (Verdict::Eligible, ModelVerdict::Usable),
            (Verdict::Preferred, ModelVerdict::Preferred),
            (Verdict::Incomparable, ModelVerdict::Incomparable),
        ];
        for (generic, expected) in cases {
            let original = decision(generic);
            let projected = project_verdict(original.clone(), &facts());
            assert_eq!(projected.verdict, expected);
            assert_eq!(projected.generic, original);
        }
        let mut one = facts();
        one.failed_hard_gate_epochs = [id(5)].into();
        assert_eq!(
            project_verdict(decision(Verdict::RejectedGate), &one).verdict,
            ModelVerdict::InsufficientEvidence
        );
        let mut stale = facts();
        stale.cost_reconciled = false;
        assert_eq!(
            project_verdict(decision(Verdict::OverBudget), &stale).verdict,
            ModelVerdict::InsufficientEvidence
        );
    }
    #[test]
    fn portfolio_never_retires_partial_and_costly_unique_model_is_specialist() {
        let role = |v, unique| RoleResult {
            role: Symbol::new("author"),
            domain: Symbol::new("code"),
            active: true,
            uniquely_capable: unique,
            decision: project_verdict(decision(v), &facts()),
        };
        assert_eq!(
            portfolio_disposition(&[role(Verdict::OverBudget, true)]),
            PortfolioDisposition::Specialist
        );
        assert_ne!(
            portfolio_disposition(&[
                role(Verdict::RejectedGate, false),
                role(Verdict::Eligible, false),
            ]),
            PortfolioDisposition::RetireCandidate
        );
        assert_eq!(
            portfolio_disposition(&[
                role(Verdict::RejectedGate, false),
                role(Verdict::RejectedGate, false)
            ]),
            PortfolioDisposition::RetireCandidate
        );
    }
    fn selection() -> Selection {
        Selection {
            id: id(8),
            subjects: vec![id(1)],
            decisive_evidence: [id(2)].into(),
            report_root: id(9),
            expiry: id(10),
            staleness: StalenessInputs {
                subject_snapshot: id(11),
                evidence_root: id(12),
                policy: id(13),
            },
        }
    }
    fn pick_facts() -> PickFacts {
        PickFacts {
            complete: true,
            bootstrap: false,
            report_only: false,
            private_approved: true,
            unresolved: false,
            quarantined: false,
            subject_current: true,
            price_current: true,
            seats_current: true,
        }
    }
    #[test]
    fn pick_rejects_each_missing_fact_and_journals_offline_selection() {
        let mut variants = Vec::new();
        for refusal in [
            PickRefusal::Incomplete,
            PickRefusal::Bootstrap,
            PickRefusal::ReportOnly,
            PickRefusal::PrivateUnapproved,
            PickRefusal::Unresolved,
            PickRefusal::Quarantined,
            PickRefusal::StaleSubject,
            PickRefusal::StalePrice,
            PickRefusal::StaleSeat,
        ] {
            let mut f = pick_facts();
            match refusal {
                PickRefusal::Incomplete => f.complete = false,
                PickRefusal::Bootstrap => f.bootstrap = true,
                PickRefusal::ReportOnly => f.report_only = true,
                PickRefusal::PrivateUnapproved => f.private_approved = false,
                PickRefusal::Unresolved => f.unresolved = true,
                PickRefusal::Quarantined => f.quarantined = true,
                PickRefusal::StaleSubject => f.subject_current = false,
                PickRefusal::StalePrice => f.price_current = false,
                PickRefusal::StaleSeat => f.seats_current = false,
            };
            variants.push((f, refusal));
        }
        let args = || {
            vec![CandidateRoute {
                candidate_revision: id(1),
                provider_seat: id(14),
                route_semantics: Symbol::new("direct"),
                driver_selection: "model-x --seat seat-a".into(),
            }]
        };
        for (f, want) in variants {
            let got = emit_pick(
                &mut PickJournal::default(),
                selection(),
                Symbol::new("author"),
                args(),
                ModelVerdict::Preferred,
                vec![],
                vec![],
                id(15),
                99,
                true,
                &f,
            )
            .unwrap_err();
            assert_eq!(got, want);
            assert!(!got.missing_fact().is_empty());
        }
        let mut journal = PickJournal::default();
        let pick = emit_pick(
            &mut journal,
            selection(),
            Symbol::new("author"),
            args(),
            ModelVerdict::Preferred,
            vec![Symbol::new("quality")],
            vec![],
            id(15),
            99,
            true,
            &pick_facts(),
        )
        .unwrap();
        assert_eq!(journal.entries(), std::slice::from_ref(&pick));
        assert_eq!(pick.driver_selection(), "model-x --seat seat-a");
        assert!(pick.rationale().contains("expires-at-ms=99"));
    }
}
