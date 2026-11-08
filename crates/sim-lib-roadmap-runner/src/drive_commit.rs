// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Durable close of one admitted effect or one pure cause.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_lib_journal::{JournalBackend, JournalHead};
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{ExecutionEvent, PhaseRunState};

use super::drive::{
    DirectionRequest, DriveResult, RunSnapshot, SnapshotRow, digest, revision_id, snapshot_page_id,
    wake_on,
};
use super::{
    CancellationPort, EffectPort, ExecutionRecord, ReadinessPort, RoadmapRunnerService,
    ServiceError, state_text,
};
use crate::Mode;

impl<B: JournalBackend, R: ReadinessPort, E: EffectPort, C: CancellationPort>
    RoadmapRunnerService<B, R, E, C>
{
    pub(super) fn finish_effect(
        &mut self,
        leaf: &PhaseId,
        effect_id: &str,
        intent_head: &JournalHead,
        event: &ExecutionEvent,
    ) -> Result<(), ServiceError> {
        let rebuilt = self.journal.rebuild()?;
        let parent = if crate::record_law::receipt_parent_is_current(&rebuilt.records, effect_id) {
            rebuilt.head
        } else {
            intent_head.clone()
        };
        let receipt_head = match self.journal.append(
            Some(&parent),
            ExecutionRecord::EffectReceipt {
                effect_id: effect_id.to_owned(),
                outcome: event.observation.kind.to_string(),
                output: None,
            },
            vec![],
        ) {
            Ok(head) => head,
            Err(error) => {
                self.uncertain = true;
                return Err(error.into());
            }
        };
        let next = match reduce_event(self, leaf, event) {
            Ok(next) => next,
            Err(error) => {
                self.note_uncertain()?;
                return Err(error);
            }
        };
        if state_text(self.transition.state) != state_text(next.state) {
            if let Err(error) = self.journal.append(
                Some(&receipt_head),
                ExecutionRecord::StateTransition {
                    from: state_text(self.transition.state).into(),
                    to: state_text(next.state).into(),
                },
                vec![],
            ) {
                self.note_uncertain()?;
                return Err(error.into());
            }
        } else if let Err(error) = self.append_cause(
            "effect-commit",
            "effect-commit",
            &digest(&[effect_id.as_bytes()]),
        ) {
            self.note_uncertain()?;
            return Err(error);
        }
        self.transition = next;
        self.transition_faithful = true;
        self.bump();
        Ok(())
    }

    pub(super) fn append_cause(
        &mut self,
        cause: &str,
        key: &str,
        after: &ContentId,
    ) -> Result<(), ServiceError> {
        let rebuilt = self.journal.rebuild()?;
        let before = rebuilt.causal.facts.get(key).cloned();
        if before.as_ref() == Some(after) {
            return Ok(());
        }
        let _ = self.journal.append_delta(
            &rebuilt.head,
            cause,
            vec![crate::SemanticChange {
                key: key.to_owned(),
                before,
                after: Some(after.clone()),
            }],
            BTreeSet::new(),
        )?;
        Ok(())
    }

    /// Records the decision and its answer in one cause.
    pub(super) fn append_direction(
        &mut self,
        decision: &ContentId,
        answer: &ContentId,
    ) -> Result<(), ServiceError> {
        let rebuilt = self.journal.rebuild()?;
        let decision_before = rebuilt.causal.facts.get("direction-decision").cloned();
        let answer_before = rebuilt.causal.facts.get("direction-answer").cloned();
        if decision_before.as_ref() == Some(decision) && answer_before.as_ref() == Some(answer) {
            return Ok(());
        }
        let _ = self.journal.append_delta(
            &rebuilt.head,
            "direction-decision",
            vec![
                crate::SemanticChange {
                    key: "direction-decision".to_owned(),
                    before: decision_before,
                    after: Some(decision.clone()),
                },
                crate::SemanticChange {
                    key: "direction-answer".to_owned(),
                    before: answer_before,
                    after: Some(answer.clone()),
                },
            ],
            BTreeSet::new(),
        )?;
        Ok(())
    }

    pub(super) fn note_uncertain(&mut self) -> Result<(), ServiceError> {
        self.uncertain = true;
        let marker = digest(&[b"uncertain"]);
        self.append_cause("effect-uncertain", "effect-uncertain", &marker)
    }

    pub(super) fn restore_execution(&mut self) -> Result<(), ServiceError> {
        let rebuilt = self.journal.rebuild()?;
        let folded = fold_execution(
            &rebuilt.records,
            &self.opening_head,
            &self.request.authority.identity.execution,
        );
        let journal_has_receipt = rebuilt
            .records
            .iter()
            .any(|record| matches!(record, ExecutionRecord::EffectReceipt { .. }));
        if let Some(state) = folded.phase {
            self.transition.state = state;
        }
        self.revision_n = folded.revision_n;
        self.revision = revision_id(&self.opening_head, self.revision_n);
        self.mode = mode_of(rebuilt.causal.facts.get("session-mode"));
        // Offers are not causal facts. A committed effect is the one resume.
        self.wait_resumed = folded.resumed;
        self.snapshots = folded.snapshots;
        let journal_phase = self.transition.state;
        let events = self.effects.receipts(&self.request.authority.identity)?;
        if events.is_empty() {
            // Port receipts rebuild the reducer. An empty list keeps the phase
            // and refuses another effect once a receipt exists.
            self.transition_faithful = !journal_has_receipt;
            return Ok(());
        }
        // A receipt the reducer rejects keeps the folded phase and the service.
        match self.replay() {
            Ok(replayed) if replayed.state == journal_phase => {
                self.transition = replayed;
                self.transition_faithful = true;
            }
            _ => self.transition_faithful = false,
        }
        Ok(())
    }

    pub(super) fn restore_direction(&mut self) -> Result<(), ServiceError> {
        let Some(question) = self.question.clone() else {
            return Ok(());
        };
        let rebuilt = self.journal.rebuild()?;
        if rebuilt.causal.facts.get("direction-request") != Some(&question.id) {
            return Ok(());
        }
        self.published = Some(DirectionRequest {
            id: question.id,
            execution: self.request.authority.identity.execution.clone(),
            expected_revision: self.revision.clone(),
            question: question.question,
            answer_kind: question.answer_kind,
            required_authority: question.required_authority,
        });
        if let Some(decision) = rebuilt.causal.facts.get("direction-decision") {
            // The answer text is not a causal id. A later reply matches the stored digest.
            self.consumed_answer = rebuilt.causal.facts.get("direction-answer").cloned();
            self.consumed = Some((decision.clone(), String::new()));
        }
        Ok(())
    }

    pub(super) fn journal_uncertain(&self) -> Result<bool, ServiceError> {
        let rebuilt = self.journal.rebuild()?;
        Ok(unresolved_effect(&rebuilt.records)
            || rebuilt.causal.facts.contains_key("effect-uncertain"))
    }

    pub(super) fn bump(&mut self) {
        self.revision_n += 1;
        self.revision = revision_id(&self.opening_head, self.revision_n);
    }

    pub(super) fn advanced(&self) -> DriveResult {
        if self.transition.state == PhaseRunState::Succeeded
            || self.transition.state == PhaseRunState::Failed
            || self.transition.state == PhaseRunState::Cancelled
        {
            DriveResult::Complete {
                seal_input: self.revision.clone(),
            }
        } else {
            DriveResult::Advanced {
                transition: self.revision.clone(),
                wake: wake_on(),
            }
        }
    }

    pub(super) fn retain_snapshot(&mut self) -> Result<(), ServiceError> {
        let before = self.journal.rebuild()?;
        let verified = self.journal.snapshot(&before.head)?;
        let after = self.journal.rebuild()?;
        if !after
            .snapshots
            .iter()
            .any(|item| item.state == verified.state)
        {
            return Err(crate::ExecutionJournalError::Semantic("snapshot state").into());
        }
        let mut rows: Vec<SnapshotRow> = before
            .causal
            .facts
            .iter()
            .map(|(key, value)| SnapshotRow {
                key: key.clone(),
                value: value.clone(),
            })
            .collect();
        rows.push(SnapshotRow {
            key: "evidence-set".into(),
            value: verified.evidence_set,
        });
        if self
            .snapshots
            .iter()
            .any(|item| item.journal_head == verified.covers)
        {
            return Ok(());
        }
        let id = snapshot_page_id(&self.snapshots, &verified.state, &verified.covers);
        self.snapshots.push(RunSnapshot {
            id,
            execution: self.request.authority.identity.execution.clone(),
            revision: self.revision.clone(),
            journal_head: verified.covers,
            rows,
        });
        Ok(())
    }
}

struct Folded {
    revision_n: u64,
    phase: Option<PhaseRunState>,
    resumed: bool,
    snapshots: Vec<RunSnapshot>,
}

fn fold_execution(
    records: &[ExecutionRecord],
    opening: &ContentId,
    execution: &sim_roadmap_exec_core::ExecutionId,
) -> Folded {
    let mut revision_n = 0u64;
    let mut phase = None;
    let mut resumed = false;
    let mut receipt_open = false;
    let mut facts = BTreeMap::new();
    let mut snapshots = Vec::new();
    for record in records {
        match record {
            ExecutionRecord::EffectReceipt { .. } => receipt_open = true,
            ExecutionRecord::StateTransition { to, .. } => {
                if let Some(state) = phase_state(to) {
                    phase = Some(state);
                }
                if receipt_open {
                    receipt_open = false;
                    revision_n += 1;
                    resumed = true;
                }
            }
            ExecutionRecord::SemanticDelta { cause, changes, .. } => {
                for change in changes {
                    match &change.after {
                        Some(after) => {
                            facts.insert(change.key.clone(), after.clone());
                        }
                        None => {
                            facts.remove(&change.key);
                        }
                    }
                }
                if cause == "direction-decision"
                    || changes.iter().any(|change| change.key == "session-mode")
                {
                    revision_n += 1;
                }
                if receipt_open && cause == "effect-commit" {
                    receipt_open = false;
                    revision_n += 1;
                    resumed = true;
                }
            }
            ExecutionRecord::Snapshot {
                covers,
                state,
                evidence_set,
            } => {
                if snapshots
                    .iter()
                    .any(|item: &RunSnapshot| item.journal_head == *covers)
                {
                    continue;
                }
                let mut rows: Vec<_> = facts
                    .iter()
                    .map(|(key, value)| SnapshotRow {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect();
                rows.push(SnapshotRow {
                    key: "evidence-set".into(),
                    value: evidence_set.clone(),
                });
                let id = snapshot_page_id(&snapshots, state, covers);
                snapshots.push(RunSnapshot {
                    id,
                    execution: execution.clone(),
                    revision: revision_id(opening, revision_n),
                    journal_head: covers.clone(),
                    rows,
                });
            }
            _ => {}
        }
    }
    Folded {
        revision_n,
        phase,
        resumed,
        snapshots,
    }
}

fn mode_of(stored: Option<&ContentId>) -> Mode {
    let Some(stored) = stored else {
        return Mode::Running;
    };
    let modes = [
        (b"paused".as_slice(), Mode::Paused),
        (b"draining".as_slice(), Mode::Draining),
        (b"stopped".as_slice(), Mode::Stopped),
        (b"running".as_slice(), Mode::Running),
    ];
    modes
        .into_iter()
        .find(|(label, _)| stored == &digest(&[*label]))
        .map(|(_, mode)| mode)
        .unwrap_or(Mode::Paused)
}

fn phase_state(text: &str) -> Option<PhaseRunState> {
    Some(match text {
        "planned" => PhaseRunState::Planned,
        "running" => PhaseRunState::Running,
        "reconciling" => PhaseRunState::Reconciling,
        "succeeded" => PhaseRunState::Succeeded,
        "failed" => PhaseRunState::Failed,
        "cancelled" => PhaseRunState::Cancelled,
        _ => return None,
    })
}

/// A request with no receipt is open. A receipt with no committed success is
/// open too, including when the uncertain marker itself was not written.
fn unresolved_effect(records: &[ExecutionRecord]) -> bool {
    let mut request_open = false;
    let mut receipt_open = false;
    for record in records {
        match record {
            ExecutionRecord::EffectRequested { .. } => {
                if request_open || receipt_open {
                    return true;
                }
                request_open = true;
            }
            ExecutionRecord::EffectReceipt { .. } => {
                if !request_open || receipt_open {
                    return true;
                }
                request_open = false;
                receipt_open = true;
            }
            ExecutionRecord::StateTransition { .. } if receipt_open => receipt_open = false,
            ExecutionRecord::SemanticDelta { cause, .. }
                if receipt_open && cause == "effect-commit" =>
            {
                receipt_open = false;
            }
            _ => {}
        }
    }
    request_open || receipt_open
}

fn reduce_event<B, R, E, C>(
    service: &RoadmapRunnerService<B, R, E, C>,
    leaf: &PhaseId,
    event: &ExecutionEvent,
) -> Result<sim_roadmap_exec_core::Transition, ServiceError>
where
    B: JournalBackend,
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    Ok(sim_roadmap_exec_core::reduce(
        &service.request.policy,
        &service.request.mutation,
        &service.request.authority.identity.execution,
        leaf,
        &service.request.attempt,
        &service.transition,
        event,
    )?)
}
