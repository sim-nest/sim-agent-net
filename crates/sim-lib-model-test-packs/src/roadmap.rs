//! Downstream-truth measurement for synthetic roadmap-writing tasks.
use std::collections::{BTreeMap, BTreeSet};
pub const ROADMAP_TASK_SCHEMA: &str = "sim.model-test-roadmap-task/v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextKind {
    None,
    Card,
    Rustdoc,
    Recipe,
    IndexRoute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoadmapFacet {
    Structure,
    Intent,
    SourceTruth,
    Reuse,
    OwnerBoundary,
    DependencySafety,
    Executability,
    ProofDelivery,
    InventedApi,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCandidate {
    pub anchor: String,
    pub owner: String,
    pub kind: ContextKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadmapTask {
    pub schema: String,
    pub id: String,
    pub pair_id: String,
    pub context: ContextKind,
    pub goal: String,
    pub conflicting_constraints: Vec<String>,
    pub active_predecessor: String,
    pub candidates: Vec<SourceCandidate>,
    pub output_contract: String,
    pub frozen_epoch: String,
    pub input_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoadmapFailure {
    Contract(&'static str),
    Source(&'static str),
}
impl RoadmapTask {
    pub fn validate(&self) -> Result<(), RoadmapFailure> {
        if self.schema != ROADMAP_TASK_SCHEMA {
            return Err(RoadmapFailure::Contract("schema"));
        }
        if [
            &self.id,
            &self.pair_id,
            &self.goal,
            &self.active_predecessor,
            &self.output_contract,
            &self.frozen_epoch,
            &self.input_digest,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
        {
            return Err(RoadmapFailure::Contract("required field"));
        }
        if self.conflicting_constraints.len() < 2 {
            return Err(RoadmapFailure::Contract("conflicting constraints"));
        }
        if self.candidates.is_empty() {
            return Err(RoadmapFailure::Source("source candidates"));
        }
        if self.frozen_epoch.contains("workbench/") || self.frozen_epoch.contains("active") {
            return Err(RoadmapFailure::Contract("live roadmap forbidden"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedPhase {
    pub standalone_intent: bool,
    pub resolved_predecessor: bool,
    pub source_anchors: Vec<String>,
    pub reuse_anchors: Vec<String>,
    pub owners: Vec<String>,
    pub dependencies_safe: bool,
    pub executable_steps: Vec<String>,
    pub proof_metadata: Option<String>,
    pub delivery_metadata: Option<String>,
    pub claimed_apis: Vec<String>,
    pub dangling_cursor: bool,
    pub closeout_owner: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeterministicGrade {
    pub failures: Vec<RoadmapFailure>,
    pub facets: BTreeMap<RoadmapFacet, bool>,
}
pub fn grade_deterministic(task: &RoadmapTask, phase: &ProposedPhase) -> DeterministicGrade {
    let mut failures = Vec::new();
    if let Err(e) = task.validate() {
        failures.push(e)
    }
    let candidates: BTreeSet<_> = task.candidates.iter().map(|c| c.anchor.as_str()).collect();
    let source = !phase.source_anchors.is_empty()
        && phase
            .source_anchors
            .iter()
            .all(|a| candidates.contains(a.as_str()));
    let owners: BTreeSet<_> = task.candidates.iter().map(|c| c.owner.as_str()).collect();
    let owner =
        !phase.owners.is_empty() && phase.owners.iter().all(|o| owners.contains(o.as_str()));
    let invented = phase
        .claimed_apis
        .iter()
        .any(|a| !candidates.contains(a.as_str()));
    let structure = !phase.dangling_cursor && phase.closeout_owner.is_some();
    let executable = !phase.executable_steps.is_empty();
    let proof = phase.proof_metadata.is_some() && phase.delivery_metadata.is_some();
    if !source {
        failures.push(RoadmapFailure::Source("unresolved source claim"))
    }
    if !owner {
        failures.push(RoadmapFailure::Source("false ownership"))
    }
    if invented {
        failures.push(RoadmapFailure::Source("invented api"))
    }
    for (ok, name) in [
        (structure, "v3 structure"),
        (phase.standalone_intent, "stand-alone intent"),
        (phase.resolved_predecessor, "predecessor reconciliation"),
        (phase.dependencies_safe, "dependency safety"),
        (executable, "phase executability"),
        (proof, "proof and delivery metadata"),
    ] {
        if !ok {
            failures.push(RoadmapFailure::Contract(name))
        }
    }
    DeterministicGrade {
        failures,
        facets: BTreeMap::from([
            (RoadmapFacet::Structure, structure),
            (RoadmapFacet::Intent, phase.standalone_intent),
            (RoadmapFacet::SourceTruth, source),
            (RoadmapFacet::Reuse, !phase.reuse_anchors.is_empty()),
            (RoadmapFacet::OwnerBoundary, owner),
            (RoadmapFacet::DependencySafety, phase.dependencies_safe),
            (RoadmapFacet::Executability, executable),
            (RoadmapFacet::ProofDelivery, proof),
            (RoadmapFacet::InventedApi, !invented),
        ]),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewDecision {
    Pass,
    Fail,
    Abstain,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlindedReview {
    pub reviewer: String,
    pub anchor_set: String,
    pub decision: ReviewDecision,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalibratedReview {
    pub decision: ReviewDecision,
    pub disagreement: bool,
    pub abstentions: u32,
}
pub fn calibrated_review(
    grade: &DeterministicGrade,
    reviews: &[BlindedReview],
) -> CalibratedReview {
    let abstentions = reviews
        .iter()
        .filter(|r| r.decision == ReviewDecision::Abstain)
        .count() as u32;
    let decisions: BTreeSet<_> = reviews
        .iter()
        .filter_map(|r| (r.decision != ReviewDecision::Abstain).then_some(r.decision))
        .collect();
    let disagreement = decisions.len() > 1;
    let judge = if disagreement || decisions.is_empty() {
        ReviewDecision::Abstain
    } else {
        *decisions.iter().next().unwrap()
    };
    CalibratedReview {
        decision: if grade.failures.is_empty() {
            judge
        } else {
            ReviewDecision::Fail
        },
        disagreement,
        abstentions,
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossPlayResult {
    pub task_id: String,
    pub calibrator: String,
    pub completed: bool,
    pub ambiguity_count: u32,
    pub repair_steps: u32,
    pub scope_escapes: u32,
    pub final_semantics_digest: String,
}
impl CrossPlayResult {
    pub fn valid(&self) -> bool {
        !self.task_id.is_empty()
            && !self.calibrator.is_empty()
            && !self.final_semantics_digest.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairOutcome {
    pub pair_id: String,
    pub baseline: i32,
    pub treatment: i32,
    pub intact: bool,
}
pub fn paired_context_uplift(xs: &[PairOutcome]) -> Option<f64> {
    let xs: Vec<_> = xs
        .iter()
        .filter(|x| x.intact && !x.pair_id.is_empty())
        .collect();
    (!xs.is_empty()).then(|| {
        xs.iter()
            .map(|x| f64::from(x.treatment - x.baseline))
            .sum::<f64>()
            / xs.len() as f64
    })
}

#[cfg(test)]
mod focused_tests {
    use super::*;
    fn task() -> RoadmapTask {
        RoadmapTask {
            schema: ROADMAP_TASK_SCHEMA.into(),
            id: "t".into(),
            pair_id: "p".into(),
            context: ContextKind::IndexRoute,
            goal: "bounded goal".into(),
            conflicting_constraints: vec!["reuse".into(), "narrow owner".into()],
            active_predecessor: "mini.01".into(),
            candidates: vec![SourceCandidate {
                anchor: "route/query".into(),
                owner: "sim-mini".into(),
                kind: ContextKind::IndexRoute,
            }],
            output_contract: "v3".into(),
            frozen_epoch: "sha256:frozen".into(),
            input_digest: "sha256:pair".into(),
        }
    }
    fn phase() -> ProposedPhase {
        ProposedPhase {
            standalone_intent: true,
            resolved_predecessor: true,
            source_anchors: vec!["route/query".into()],
            reuse_anchors: vec!["route/query".into()],
            owners: vec!["sim-mini".into()],
            dependencies_safe: true,
            executable_steps: vec!["compose".into()],
            proof_metadata: Some("workspace".into()),
            delivery_metadata: Some("deferred".into()),
            claimed_apis: vec!["route/query".into()],
            dangling_cursor: false,
            closeout_owner: Some("sim-mini".into()),
        }
    }
    #[test]
    fn truth_and_contract_precede_judgment() {
        let mut p = phase();
        p.owners = vec!["sim-kernel".into()];
        let g = grade_deterministic(&task(), &p);
        assert_eq!(
            calibrated_review(
                &g,
                &[BlindedReview {
                    reviewer: "r".into(),
                    anchor_set: "fixed".into(),
                    decision: ReviewDecision::Pass
                }]
            )
            .decision,
            ReviewDecision::Fail
        );
    }
    #[test]
    fn broken_pairs_do_not_bias_uplift() {
        assert_eq!(
            paired_context_uplift(&[
                PairOutcome {
                    pair_id: "a".into(),
                    baseline: 1,
                    treatment: 3,
                    intact: true
                },
                PairOutcome {
                    pair_id: "b".into(),
                    baseline: 0,
                    treatment: 99,
                    intact: false
                }
            ]),
            Some(2.0)
        );
    }
    #[test]
    fn live_material_is_refused() {
        let mut t = task();
        t.frozen_epoch = "docs/workbench/active".into();
        assert!(t.validate().is_err());
    }
}
