//! Responsive coding-assistance sessions and deterministic semantic grading.

use std::collections::{BTreeMap, BTreeSet};

pub const ASSISTANCE_SESSION_SCHEMA: &str = "sim.model-test-coding-assistance/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssistanceKind {
    Diagnosis,
    Explanation,
    MinimalEdit,
    Completion,
    CodeReview,
    TestSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedResponse {
    AskOneQuestion,
    Act,
    ExplainOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steering {
    None,
    Replace,
    Extend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssistanceFacet {
    FirstUsefulEvent,
    FirstValidAction,
    FeedbackCorrection,
    TerminalQuality,
    UnnecessaryEdits,
    VerbosityCost,
    TotalResources,
    Steering,
    Authorization,
    FrozenWorkspace,
    HumanUsefulness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistanceSession {
    pub schema: String,
    pub id: String,
    pub kind: AssistanceKind,
    pub frozen_epoch: String,
    pub public_source_digest: String,
    pub expected: ExpectedResponse,
    pub steering: Steering,
    pub edit_authorized: bool,
    pub interaction_ceiling_ms: Option<u64>,
    pub hidden_grader_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicAssistanceSession {
    pub id: String,
    pub kind: AssistanceKind,
    pub frozen_epoch: String,
    pub public_source_digest: String,
}

impl AssistanceSession {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != ASSISTANCE_SESSION_SCHEMA {
            return Err("schema");
        }
        if self.id.trim().is_empty()
            || self.public_source_digest.trim().is_empty()
            || self.hidden_grader_id.trim().is_empty()
        {
            return Err("required input");
        }
        if self.frozen_epoch.contains("workbench/") || self.frozen_epoch.contains("active") {
            return Err("live workspace forbidden");
        }
        if !self.edit_authorized && matches!(self.expected, ExpectedResponse::Act) {
            return Err("action requires edit authority");
        }
        Ok(())
    }

    /// Public trial material contains neither hidden grader identity nor policy bytes.
    pub fn export_public(&self) -> Result<PublicAssistanceSession, &'static str> {
        self.validate()?;
        Ok(PublicAssistanceSession {
            id: self.id.clone(),
            kind: self.kind,
            frozen_epoch: self.frozen_epoch.clone(),
            public_source_digest: self.public_source_digest.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistanceTrace {
    pub first_useful_ms: Option<u64>,
    pub first_valid_action_ms: Option<u64>,
    pub correction_ms: Option<u64>,
    pub deterministic_clock: bool,
    pub asked_questions: u32,
    pub edit_paths: BTreeSet<String>,
    pub necessary_edit_paths: BTreeSet<String>,
    pub terminal_quality: u16,
    pub usefulness_anchor: u16,
    pub output_bytes: u64,
    pub useful_output_bytes: u64,
    pub tool_calls: u32,
    pub resource_units: u64,
    pub correct: bool,
    pub preserved_finished_work: bool,
    pub followed_steering: Steering,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistanceGrade {
    pub eligible: bool,
    pub service_latency_ms: Option<u64>,
    pub facets: BTreeMap<AssistanceFacet, u64>,
    pub failures: Vec<&'static str>,
}

pub fn grade_assistance(session: &AssistanceSession, trace: &AssistanceTrace) -> AssistanceGrade {
    let mut failures = session.validate().err().into_iter().collect::<Vec<_>>();
    let response_ok = match session.expected {
        ExpectedResponse::AskOneQuestion => {
            trace.asked_questions == 1 && trace.edit_paths.is_empty()
        }
        ExpectedResponse::Act => trace.asked_questions == 0 && !trace.edit_paths.is_empty(),
        ExpectedResponse::ExplainOnly => trace.asked_questions == 0 && trace.edit_paths.is_empty(),
    };
    let authorization = session.edit_authorized || trace.edit_paths.is_empty();
    let restraint = trace.edit_paths.is_subset(&trace.necessary_edit_paths);
    let steering = trace.followed_steering == session.steering
        && (session.steering == Steering::None || trace.preserved_finished_work);
    let ceiling = session.interaction_ceiling_ms.is_none_or(|limit| {
        trace
            .first_useful_ms
            .is_some_and(|elapsed| elapsed <= limit)
    });
    for (ok, name) in [
        (trace.correct, "incorrect"),
        (response_ok, "wrong response mode"),
        (authorization, "unauthorized edit"),
        (restraint, "unnecessary edit"),
        (steering, "incorrect steering"),
        (trace.deterministic_clock, "nondeterministic clock"),
        (ceiling, "interaction ceiling"),
    ] {
        if !ok {
            failures.push(name);
        }
    }
    let verbosity = trace.output_bytes.saturating_sub(trace.useful_output_bytes);
    AssistanceGrade {
        eligible: failures.is_empty(),
        service_latency_ms: trace.first_useful_ms,
        facets: BTreeMap::from([
            (
                AssistanceFacet::FirstUsefulEvent,
                trace.first_useful_ms.unwrap_or(u64::MAX),
            ),
            (
                AssistanceFacet::FirstValidAction,
                trace.first_valid_action_ms.unwrap_or(u64::MAX),
            ),
            (
                AssistanceFacet::FeedbackCorrection,
                trace.correction_ms.unwrap_or(u64::MAX),
            ),
            (
                AssistanceFacet::TerminalQuality,
                u64::from(trace.terminal_quality),
            ),
            (
                AssistanceFacet::UnnecessaryEdits,
                trace
                    .edit_paths
                    .difference(&trace.necessary_edit_paths)
                    .count() as u64,
            ),
            (AssistanceFacet::VerbosityCost, verbosity),
            (
                AssistanceFacet::TotalResources,
                trace.resource_units + u64::from(trace.tool_calls),
            ),
            (AssistanceFacet::Steering, u64::from(steering)),
            (AssistanceFacet::Authorization, u64::from(authorization)),
            (
                AssistanceFacet::FrozenWorkspace,
                u64::from(trace.deterministic_clock),
            ),
            (
                AssistanceFacet::HumanUsefulness,
                u64::from(trace.usefulness_anchor),
            ),
        ]),
        failures,
    }
}

/// Correctness gates ranking. Latency only breaks ties between eligible traces.
pub fn prefer_assistance(a: &AssistanceGrade, b: &AssistanceGrade) -> std::cmp::Ordering {
    a.eligible
        .cmp(&b.eligible)
        .then_with(|| {
            a.facets[&AssistanceFacet::TerminalQuality]
                .cmp(&b.facets[&AssistanceFacet::TerminalQuality])
        })
        .then_with(|| {
            b.service_latency_ms
                .unwrap_or(u64::MAX)
                .cmp(&a.service_latency_ms.unwrap_or(u64::MAX))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(kind: AssistanceKind, expected: ExpectedResponse) -> AssistanceSession {
        AssistanceSession {
            schema: ASSISTANCE_SESSION_SCHEMA.into(),
            id: format!("{kind:?}"),
            kind,
            frozen_epoch: "sha256:frozen-workspace".into(),
            public_source_digest: "sha256:source".into(),
            expected,
            steering: Steering::None,
            edit_authorized: expected == ExpectedResponse::Act,
            interaction_ceiling_ms: None,
            hidden_grader_id: "private:grader-16".into(),
        }
    }
    fn trace() -> AssistanceTrace {
        AssistanceTrace {
            first_useful_ms: Some(30),
            first_valid_action_ms: Some(40),
            correction_ms: Some(10),
            deterministic_clock: true,
            asked_questions: 0,
            edit_paths: BTreeSet::from(["src/lib.rs".into()]),
            necessary_edit_paths: BTreeSet::from(["src/lib.rs".into()]),
            terminal_quality: 90,
            usefulness_anchor: 4,
            output_bytes: 120,
            useful_output_bytes: 100,
            tool_calls: 2,
            resource_units: 8,
            correct: true,
            preserved_finished_work: true,
            followed_steering: Steering::None,
        }
    }
    #[test]
    fn six_roles_are_distinct_short_sessions() {
        let kinds = [
            AssistanceKind::Diagnosis,
            AssistanceKind::Explanation,
            AssistanceKind::MinimalEdit,
            AssistanceKind::Completion,
            AssistanceKind::CodeReview,
            AssistanceKind::TestSelection,
        ];
        assert_eq!(kinds.into_iter().collect::<BTreeSet<_>>().len(), 6);
    }
    #[test]
    fn ambiguity_requires_one_question_but_scoped_work_requires_action() {
        let mut ambiguous = session(AssistanceKind::Diagnosis, ExpectedResponse::AskOneQuestion);
        ambiguous.edit_authorized = false;
        let mut t = trace();
        t.asked_questions = 1;
        t.edit_paths.clear();
        assert!(grade_assistance(&ambiguous, &t).eligible);
        let scoped = session(AssistanceKind::MinimalEdit, ExpectedResponse::Act);
        t.asked_questions = 1;
        assert!(!grade_assistance(&scoped, &t).eligible);
    }
    #[test]
    fn steering_and_finished_work_are_graded() {
        let mut s = session(AssistanceKind::Completion, ExpectedResponse::Act);
        s.steering = Steering::Replace;
        let mut t = trace();
        t.followed_steering = Steering::Replace;
        assert!(grade_assistance(&s, &t).eligible);
        t.preserved_finished_work = false;
        assert!(!grade_assistance(&s, &t).eligible);
        s.steering = Steering::Extend;
        t.followed_steering = Steering::Extend;
        t.preserved_finished_work = true;
        assert!(grade_assistance(&s, &t).eligible);
    }
    #[test]
    fn diagnosis_cannot_edit_and_minimal_edit_rejects_cleanup() {
        let s = session(AssistanceKind::Diagnosis, ExpectedResponse::ExplainOnly);
        assert!(!grade_assistance(&s, &trace()).eligible);
        let s = session(AssistanceKind::MinimalEdit, ExpectedResponse::Act);
        let mut t = trace();
        t.edit_paths.insert("README.md".into());
        assert!(!grade_assistance(&s, &t).eligible);
    }
    #[test]
    fn fast_wrong_never_outranks_slower_correct() {
        let s = session(AssistanceKind::MinimalEdit, ExpectedResponse::Act);
        let good = grade_assistance(&s, &trace());
        let mut bad_trace = trace();
        bad_trace.first_useful_ms = Some(1);
        bad_trace.correct = false;
        assert_eq!(
            prefer_assistance(&good, &grade_assistance(&s, &bad_trace)),
            std::cmp::Ordering::Greater
        );
    }
    #[test]
    fn latency_is_service_evidence_unless_ceiling_is_declared() {
        let mut s = session(AssistanceKind::MinimalEdit, ExpectedResponse::Act);
        let mut t = trace();
        t.first_useful_ms = Some(50_000);
        assert!(grade_assistance(&s, &t).eligible);
        s.interaction_ceiling_ms = Some(100);
        assert!(!grade_assistance(&s, &t).eligible);
    }
    #[test]
    fn facets_remain_separate_and_frozen_inputs_are_required() {
        let s = session(AssistanceKind::MinimalEdit, ExpectedResponse::Act);
        let g = grade_assistance(&s, &trace());
        assert_eq!(g.facets.len(), 11);
        let mut live = s;
        live.frozen_epoch = "docs/workbench/active".into();
        assert!(!grade_assistance(&live, &trace()).eligible);
    }

    #[test]
    fn public_export_is_digest_only_and_omits_hidden_policy() {
        let s = session(AssistanceKind::CodeReview, ExpectedResponse::ExplainOnly);
        let public = s.export_public().unwrap();
        assert_eq!(public.public_source_digest, "sha256:source");
        assert!(!format!("{public:?}").contains("private:grader-16"));
    }
}
