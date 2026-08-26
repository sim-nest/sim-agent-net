//! Model-trial adapter for the domain-neutral study lifecycle.
//!
//! The adapter owns orchestration and evidence normalization only. Actual BRIDGE,
//! runner, agent-effect, and sandbox work crosses [`TrialBackend`], so replay can
//! prove that none of those effectful dependencies were entered.

use crate::{ContentId as TrialContentId, FacetObservation};
use sim_kernel::ContentId;
use sim_lib_study::{AttemptEvidence, Cancellation, RequiredClosure, StudyExecutor};
use sim_study_core::{AttemptOutcome, StudyCoordinate};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialKind {
    SingleTurn,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedBounds {
    pub deadline_ms: u64,
    pub max_retries: u32,
    pub max_requests: u32,
    pub max_steps: u32,
    pub max_work_units: u32,
    pub max_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialPlan {
    pub kind: TrialKind,
    pub coordinate: ContentId,
    pub provider_seat: String,
    pub ask_packet: Vec<u8>,
    pub warrants: Vec<String>,
    pub context_ids: Vec<TrialContentId>,
    pub output_shape: String,
    pub sampling: String,
    pub rooted_workspace: Option<String>,
    pub public_checks: Vec<Vec<String>>,
    pub hidden_grading_domain: TrialContentId,
    pub bounds: SealedBounds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureAttribution {
    Provider,
    Broker,
    Observer,
    Grader,
    Sandbox,
    Host,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationalFailure {
    pub attribution: FailureAttribution,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialTrace {
    pub checked_terminal_face: Vec<u8>,
    pub packets: Vec<TrialContentId>,
    pub effects: Vec<TrialContentId>,
    pub denials: Vec<String>,
    pub workspace_mutations: Vec<TrialContentId>,
    pub checks: Vec<TrialContentId>,
    pub sandbox_reports: Vec<TrialContentId>,
    pub usage: UsageRecord,
    pub recoveries: Vec<String>,
    pub finish_state: String,
    pub refusal: Option<String>,
    pub malformed_outputs: u32,
    pub repairs: u32,
    pub truncated: bool,
    pub stop_reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrialObservation {
    pub trace: TrialTrace,
    pub facets: Vec<FacetObservation>,
}

pub trait TrialBackend: Send {
    /// Executes through the checked BRIDGE/model or BRIDGE/agent stack. Security-
    /// bearing workspace commands must be handed to the sandbox by this port.
    fn execute(
        &mut self,
        plan: &TrialPlan,
        cancellation: &dyn Cancellation,
    ) -> Result<TrialObservation, OperationalFailure>;
    fn cancel(&mut self, claim: &ContentId);
}

#[derive(Clone, Debug, PartialEq)]
pub enum CassetteEntry {
    Observed(Box<TrialObservation>),
    Operational(OperationalFailure),
}

pub trait TrialResolver: Send + Sync {
    fn plan(&self, coordinate: &StudyCoordinate) -> Result<TrialPlan, OperationalFailure>;
    fn closure(&self, coordinate: &StudyCoordinate) -> RequiredClosure;
}

pub struct ModelStudyExecutor<R, B> {
    resolver: R,
    backend: B,
    cassettes: BTreeMap<ContentId, CassetteEntry>,
    cancelled: BTreeSet<ContentId>,
}

impl<R, B> ModelStudyExecutor<R, B> {
    pub fn new(resolver: R, backend: B) -> Self {
        Self {
            resolver,
            backend,
            cassettes: BTreeMap::new(),
            cancelled: BTreeSet::new(),
        }
    }
    pub fn with_cassette(mut self, coordinate: ContentId, entry: CassetteEntry) -> Self {
        self.cassettes.insert(coordinate, entry);
        self
    }
}

fn encode_observation(value: &TrialObservation) -> Vec<Vec<u8>> {
    let mut objects = vec![value.trace.checked_terminal_face.clone()];
    objects.extend(
        value
            .trace
            .packets
            .iter()
            .map(|v| v.to_string().into_bytes()),
    );
    objects.extend(value.trace.effects.iter().map(|v| v.as_bytes().to_vec()));
    objects.extend(value.trace.denials.iter().map(|v| v.as_bytes().to_vec()));
    objects.extend(
        value
            .trace
            .workspace_mutations
            .iter()
            .map(|v| v.as_bytes().to_vec()),
    );
    objects.extend(value.trace.checks.iter().map(|v| v.as_bytes().to_vec()));
    objects.extend(
        value
            .trace
            .sandbox_reports
            .iter()
            .map(|v| v.as_bytes().to_vec()),
    );
    objects.push(format!("usage:{:?}", value.trace.usage).into_bytes());
    objects.push(
        format!(
            "finish:{};stop:{};repairs:{};malformed:{};truncated:{}",
            value.trace.finish_state,
            value.trace.stop_reason,
            value.trace.repairs,
            value.trace.malformed_outputs,
            value.trace.truncated
        )
        .into_bytes(),
    );
    objects.extend(
        value
            .facets
            .iter()
            .map(|v| format!("facet:{}:{}:{}", v.facet, v.score, v.provenance).into_bytes()),
    );
    objects
}

impl<R: TrialResolver, B: TrialBackend> StudyExecutor for ModelStudyExecutor<R, B> {
    fn execute(
        &mut self,
        coordinate: &StudyCoordinate,
        claim: &ContentId,
        cancellation: &dyn Cancellation,
    ) -> AttemptEvidence {
        let coordinate_id = coordinate
            .content_id()
            .expect("sealed coordinate is canonical");
        let closure = self.resolver.closure(coordinate);
        if cancellation.is_cancelled() || self.cancelled.contains(claim) {
            return AttemptEvidence {
                coordinate: coordinate_id,
                claim: claim.clone(),
                closure,
                outcome: AttemptOutcome::Unresolved,
                objects: vec![b"cancelled".to_vec()],
                retryable: false,
            };
        }
        let result = if let Some(entry) = self.cassettes.get(&coordinate_id) {
            match entry {
                CassetteEntry::Observed(v) => Ok(v.as_ref().clone()),
                CassetteEntry::Operational(e) => Err(e.clone()),
            }
        } else {
            self.resolver.plan(coordinate).and_then(|plan| {
                assert_eq!(
                    plan.coordinate, coordinate_id,
                    "trial plan must bind the complete coordinate"
                );
                assert!(
                    plan.bounds.deadline_ms > 0 && plan.bounds.max_requests > 0,
                    "sealed bounds must govern every route"
                );
                self.backend.execute(&plan, cancellation)
            })
        };
        match result {
            Ok(value) => AttemptEvidence {
                coordinate: coordinate_id,
                claim: claim.clone(),
                closure,
                outcome: AttemptOutcome::Observed,
                objects: encode_observation(&value),
                retryable: false,
            },
            Err(error) => AttemptEvidence {
                coordinate: coordinate_id,
                claim: claim.clone(),
                closure,
                outcome: AttemptOutcome::Unresolved,
                objects: vec![
                    format!("operational:{:?}:{}", error.attribution, error.message).into_bytes(),
                ],
                retryable: error.retryable,
            },
        }
    }
    fn cancel(&mut self, claim: &ContentId) {
        self.cancelled.insert(claim.clone());
        self.backend.cancel(claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EvidenceClass, FacetObservation};
    use sim_kernel::{Datum, Symbol};
    use sim_study_core::SubjectRevision;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn cid(label: &str) -> ContentId {
        Datum::String(label.into()).content_id().unwrap()
    }
    fn coordinate() -> StudyCoordinate {
        StudyCoordinate::new(
            SubjectRevision::new(cid("subject")),
            cid("task"),
            cid("harness"),
            cid("request"),
            cid("treatment"),
            7,
        )
    }
    struct NeverCancel;
    impl Cancellation for NeverCancel {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    #[derive(Clone)]
    struct Resolver {
        kind: TrialKind,
        ambient_ms: u64,
    }
    impl TrialResolver for Resolver {
        fn plan(&self, c: &StudyCoordinate) -> Result<TrialPlan, OperationalFailure> {
            Ok(TrialPlan {
                kind: self.kind,
                coordinate: c.content_id().unwrap(),
                provider_seat: "provider/fixture:seat-a".into(),
                ask_packet: b"BRIDGE ASK TX/RX".to_vec(),
                warrants: vec!["model/run".into()],
                context_ids: vec!["sha256:context".into()],
                output_shape: "model-test/terminal".into(),
                sampling: "seed=7,temp=0".into(),
                rooted_workspace: (self.kind == TrialKind::Agent).then(|| "workspace/root".into()),
                public_checks: vec![vec!["cargo".into(), "test".into()]],
                hidden_grading_domain: "sha256:hidden".into(),
                bounds: SealedBounds {
                    deadline_ms: 30_000.max(self.ambient_ms),
                    max_retries: 2,
                    max_requests: 8,
                    max_steps: 6,
                    max_work_units: 12,
                    max_tokens: 4096,
                },
            })
        }
        fn closure(&self, c: &StudyCoordinate) -> RequiredClosure {
            RequiredClosure {
                task: c.task().clone(),
                harness: c.harness().clone(),
                request: c.request().clone(),
                grader: cid("grader"),
            }
        }
    }
    struct Backend {
        calls: Arc<AtomicUsize>,
        wrong_workspace: bool,
    }
    impl TrialBackend for Backend {
        fn execute(
            &mut self,
            plan: &TrialPlan,
            _: &dyn Cancellation,
        ) -> Result<TrialObservation, OperationalFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(plan.bounds.deadline_ms, 30_000); // ambient 250ms must never leak into a turn
            let correct = !self.wrong_workspace;
            let facets = [
                "correctness",
                "protocol-discipline",
                "tool-discipline",
                "authority-recovery",
                "safety",
                "latency",
                "efficiency",
            ]
            .into_iter()
            .map(|name| FacetObservation {
                facet: name.into(),
                score: if name == "correctness" && !correct {
                    0.0
                } else {
                    1.0
                },
                passed: name != "correctness" || correct,
                reason: "independent facet".into(),
                evidence_class: EvidenceClass::Deterministic,
                provenance: format!("sha256:{name}"),
            })
            .collect();
            Ok(TrialObservation {
                trace: TrialTrace {
                    checked_terminal_face: b"checked terminal face".to_vec(),
                    packets: vec!["sha256:packet".into()],
                    effects: vec!["sha256:effect".into()],
                    denials: vec!["exec denied".into()],
                    workspace_mutations: vec!["sha256:mutation".into()],
                    checks: vec!["sha256:check".into()],
                    sandbox_reports: vec!["sha256:requested-achieved".into()],
                    usage: UsageRecord {
                        input_tokens: 10,
                        output_tokens: 4,
                        latency_ms: 3,
                    },
                    recoveries: vec!["bounded repair".into()],
                    finish_state: "explicit-finish".into(),
                    refusal: None,
                    malformed_outputs: 1,
                    repairs: 1,
                    truncated: false,
                    stop_reason: Symbol::new("stop").to_string(),
                },
                facets,
            })
        }
        fn cancel(&mut self, _: &ContentId) {}
    }
    #[test]
    fn fake_routes_normalize_and_agent_evidence_stays_distinct() {
        let c = coordinate();
        let calls = Arc::new(AtomicUsize::new(0));
        for kind in [TrialKind::SingleTurn, TrialKind::Agent] {
            let mut e = ModelStudyExecutor::new(
                Resolver {
                    kind,
                    ambient_ms: 250,
                },
                Backend {
                    calls: calls.clone(),
                    wrong_workspace: false,
                },
            );
            let out = e.execute(&c, &cid("claim"), &NeverCancel);
            assert_eq!(out.outcome, AttemptOutcome::Observed);
            assert!(out.objects.iter().any(|v| v == b"exec denied"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn cassette_replay_has_zero_effects_and_binds_complete_coordinate() {
        let c = coordinate();
        let id = c.content_id().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observation = Backend {
            calls: Arc::new(AtomicUsize::new(0)),
            wrong_workspace: false,
        }
        .execute(
            &Resolver {
                kind: TrialKind::Agent,
                ambient_ms: 250,
            }
            .plan(&c)
            .unwrap(),
            &NeverCancel,
        )
        .unwrap();
        let mut e = ModelStudyExecutor::new(
            Resolver {
                kind: TrialKind::Agent,
                ambient_ms: 250,
            },
            Backend {
                calls: calls.clone(),
                wrong_workspace: false,
            },
        )
        .with_cassette(id, CassetteEntry::Observed(Box::new(observation)));
        assert_eq!(
            e.execute(&c, &cid("claim"), &NeverCancel).outcome,
            AttemptOutcome::Observed
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn convincing_finish_cannot_hide_wrong_workspace_and_failures_are_operational() {
        let c = coordinate();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut backend = Backend {
            calls,
            wrong_workspace: true,
        };
        let value = backend
            .execute(
                &Resolver {
                    kind: TrialKind::Agent,
                    ambient_ms: 250,
                }
                .plan(&c)
                .unwrap(),
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(value.trace.finish_state, "explicit-finish");
        assert!(
            !value
                .facets
                .iter()
                .find(|v| v.facet == "correctness")
                .unwrap()
                .passed
        );
        let failure = CassetteEntry::Operational(OperationalFailure {
            attribution: FailureAttribution::Sandbox,
            message: "host unavailable".into(),
            retryable: true,
        });
        let mut e = ModelStudyExecutor::new(
            Resolver {
                kind: TrialKind::Agent,
                ambient_ms: 250,
            },
            Backend {
                calls: Arc::new(AtomicUsize::new(0)),
                wrong_workspace: false,
            },
        )
        .with_cassette(c.content_id().unwrap(), failure);
        let out = e.execute(&c, &cid("claim"), &NeverCancel);
        assert_eq!(out.outcome, AttemptOutcome::Unresolved);
        assert!(out.retryable);
        assert!(String::from_utf8_lossy(&out.objects[0]).contains("Sandbox"));
    }
}
