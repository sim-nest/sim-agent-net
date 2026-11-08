// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Conduct steps for one open execution.

use sim_lib_journal::JournalBackend;
use sim_roadmap_exec_core::{ExecutionEvent, PhaseRunState};
use sim_work_core::CapabilityGrant;

use super::drive::{
    DirectionKind, DirectionReply, DirectionRequest, DriveResult, EffectTicket, InputBatch,
    Prepared, control_of, digest, offered_resource, shapes_admit, wake_on,
};
use super::{
    CancellationPort, EffectPort, ExecutionRecord, ReadinessPort, RoadmapRunnerService,
    ServiceError,
};
use crate::{ControlIntent, Mode, WakeEvent};

enum Step {
    Quiescent,
    NeedDirection,
    Publish,
    Accept(Box<DirectionReply>),
    Note(Box<DirectionReply>),
    Pause,
    Drain,
    Resume,
    Stop,
    ResumeWait,
    Perform,
}

fn done(result: DriveResult) -> Prepared {
    Prepared::Done(Box::new(result))
}

fn admits_transition(inputs: &InputBatch) -> bool {
    inputs.conduct || positive_offer(inputs)
}

fn positive_offer(inputs: &InputBatch) -> bool {
    inputs.wakes.iter().any(|wake| {
        matches!(
            wake,
            WakeEvent::ResourceOffer { amount, .. } if *amount > 0
        )
    })
}

impl<B: JournalBackend, R: ReadinessPort, E: EffectPort, C: CancellationPort>
    RoadmapRunnerService<B, R, E, C>
{
    /// Admits at most one effect or applies one pure transition.
    ///
    /// An effect admission returns before the port runs. The caller invokes
    /// `effect_port` with no borrow of this service, then `commit_effect`.
    pub fn prepare_effect(
        &mut self,
        inputs: &InputBatch,
        grant: &CapabilityGrant,
    ) -> Result<Prepared, ServiceError> {
        if self.uncertain {
            return Err(ServiceError::EffectUncertain);
        }
        if self.pending_effect.is_some() {
            return self.prepare_while_outstanding(inputs);
        }
        if self.journal_uncertain()? {
            self.uncertain = true;
            return Err(ServiceError::EffectUncertain);
        }
        if self
            .cancellation
            .cancellation_requested(&self.request.authority.identity.execution)
        {
            let result = if self.mode == Mode::Stopped {
                DriveResult::Quiescent { wake_on: wake_on() }
            } else {
                self.stop_one()?
            };
            self.last_drive = result.clone();
            return Ok(done(result));
        }
        let step = self.classify(inputs, grant)?;
        if matches!(step, Step::Perform | Step::ResumeWait) {
            return self.begin_effect(inputs);
        }
        let result = match step {
            Step::Quiescent => self.idle(inputs),
            Step::NeedDirection => self.need_direction(),
            Step::Publish => self.publish_direction()?,
            Step::Accept(reply) => self.accept_direction(*reply)?,
            Step::Note(reply) => self.note_direction(*reply),
            Step::Pause => self.hold(Mode::Paused, "paused")?,
            Step::Drain => self.hold(Mode::Draining, "draining")?,
            Step::Resume => self.hold(Mode::Running, "running")?,
            Step::Stop => self.stop_one()?,
            Step::ResumeWait | Step::Perform => {
                return Err(ServiceError::EffectOutstanding);
            }
        };
        self.last_drive = result.clone();
        Ok(done(result))
    }

    /// Control still applies while an admitted effect has not committed.
    fn prepare_while_outstanding(&mut self, inputs: &InputBatch) -> Result<Prepared, ServiceError> {
        if self
            .cancellation
            .cancellation_requested(&self.request.authority.identity.execution)
        {
            let result = if self.mode == Mode::Stopped {
                DriveResult::Quiescent { wake_on: wake_on() }
            } else {
                self.stop_one()?
            };
            self.last_drive = result.clone();
            return Ok(done(result));
        }
        let Some(intent) = control_of(inputs) else {
            return Err(ServiceError::EffectOutstanding);
        };
        let result = match self.classify_control(intent) {
            Step::Pause => self.hold(Mode::Paused, "paused")?,
            Step::Drain => self.hold(Mode::Draining, "draining")?,
            Step::Resume => self.hold(Mode::Running, "running")?,
            Step::Stop => self.stop_one()?,
            Step::Quiescent => self.idle(inputs),
            Step::Perform
            | Step::ResumeWait
            | Step::NeedDirection
            | Step::Publish
            | Step::Accept(_)
            | Step::Note(_) => {
                return Err(ServiceError::EffectOutstanding);
            }
        };
        self.last_drive = result.clone();
        Ok(done(result))
    }

    /// Records the event from an invoke that did not borrow this service.
    pub fn commit_effect(
        &mut self,
        ticket: &EffectTicket,
        event: Result<ExecutionEvent, ServiceError>,
        inputs: &InputBatch,
    ) -> Result<DriveResult, ServiceError> {
        if self.pending_effect.as_deref() != Some(ticket.effect_id.as_str()) {
            return Err(ServiceError::EffectOutstanding);
        }
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                self.pending_effect = None;
                self.uncertain = true;
                return Err(error);
            }
        };
        self.finish_effect(&ticket.leaf, &ticket.effect_id, &ticket.intent_head, &event)?;
        self.pending_effect = None;
        self.wait_resumed = true;
        self.remember_offer(inputs);
        self.retain_snapshot()?;
        let result = self.advanced();
        self.last_drive = result.clone();
        Ok(result)
    }

    fn classify(&self, inputs: &InputBatch, grant: &CapabilityGrant) -> Result<Step, ServiceError> {
        if let Some(reply) = &inputs.reply {
            return self.classify_reply(reply, grant);
        }
        if let Some(intent) = control_of(inputs) {
            return Ok(self.classify_control(intent));
        }
        if self.missing_begin() {
            if admits_transition(inputs) {
                return Err(ServiceError::MissingBegin);
            }
            return Ok(Step::Quiescent);
        }
        if self.question.is_some() && self.published.is_none() {
            if admits_transition(inputs) {
                return Ok(Step::Publish);
            }
            return Ok(Step::Quiescent);
        }
        if self.published.is_some() && self.consumed.is_none() {
            return Ok(Step::NeedDirection);
        }
        if self.mode != Mode::Running {
            return Ok(Step::Quiescent);
        }
        if self.waiting() {
            return Ok(
                if offered_resource(inputs, &self.resources, &self.seen_offers) {
                    Step::ResumeWait
                } else {
                    Step::Quiescent
                },
            );
        }
        if !inputs.conduct {
            return Ok(Step::Quiescent);
        }
        Ok(Step::Perform)
    }

    fn classify_reply(
        &self,
        reply: &DirectionReply,
        grant: &CapabilityGrant,
    ) -> Result<Step, ServiceError> {
        let Some(request) = &self.published else {
            return Err(ServiceError::WrongDirection);
        };
        if reply.request != request.id || reply.execution != request.execution {
            return Err(ServiceError::WrongDirection);
        }
        if reply.answer_kind == DirectionKind::Advice
            || request.answer_kind == DirectionKind::Advice
        {
            return Err(ServiceError::AdvisoryDirection);
        }
        if !shapes_admit(request.answer_kind, reply.answer_kind) {
            return Err(ServiceError::WrongDirection);
        }
        if let Some((decision, answer)) = &self.consumed {
            if decision == &reply.decision && self.same_answer(answer, &reply.answer) {
                return Ok(Step::Quiescent);
            }
            return Err(ServiceError::ReplayedDirection);
        }
        if reply.expected_revision != request.expected_revision
            || reply.expected_revision != self.revision
        {
            return Err(ServiceError::StaleDirection);
        }
        if request.required_authority.is_none() {
            return Ok(Step::Note(Box::new(reply.clone())));
        }
        let authority = request.required_authority.as_ref();
        if authority.is_some_and(|item| !grant.capabilities.iter().any(|cap| cap == item)) {
            return Err(ServiceError::DirectionAuthority);
        }
        Ok(Step::Accept(Box::new(reply.clone())))
    }

    /// A blank retained answer matches only a stored digest.
    fn same_answer(&self, stored: &str, offered: &str) -> bool {
        if !stored.is_empty() {
            return stored == offered;
        }
        match &self.consumed_answer {
            Some(saved) => saved == &digest(&[offered.as_bytes()]),
            None => false,
        }
    }

    fn classify_control(&self, intent: ControlIntent) -> Step {
        match intent {
            ControlIntent::Stop if self.mode != Mode::Stopped => Step::Stop,
            ControlIntent::Pause if self.mode != Mode::Paused => Step::Pause,
            ControlIntent::Drain if self.mode != Mode::Draining => Step::Drain,
            ControlIntent::Resume if self.mode != Mode::Running => Step::Resume,
            _ => Step::Quiescent,
        }
    }

    fn missing_begin(&self) -> bool {
        self.handoff
            .as_ref()
            .is_some_and(|item| item.begin.is_none())
    }

    fn waiting(&self) -> bool {
        !self.resources.is_empty() && !self.wait_resumed
    }

    fn idle(&self, inputs: &InputBatch) -> DriveResult {
        if self.mode == Mode::Running && self.waiting() && inputs.conduct {
            DriveResult::Blocked {
                needs: self.resources.clone(),
            }
        } else {
            DriveResult::Quiescent { wake_on: wake_on() }
        }
    }

    fn need_direction(&self) -> DriveResult {
        match &self.published {
            Some(request) => DriveResult::NeedDirection {
                request: request.clone(),
            },
            None => DriveResult::Quiescent { wake_on: wake_on() },
        }
    }

    fn publish_direction(&mut self) -> Result<DriveResult, ServiceError> {
        let Some(question) = self.question.clone() else {
            return Ok(DriveResult::Quiescent { wake_on: wake_on() });
        };
        let already = self
            .journal
            .rebuild()?
            .causal
            .facts
            .get("direction-request")
            == Some(&question.id);
        if !already {
            self.append_cause("direction-request", "direction-request", &question.id)?;
        }
        let request = DirectionRequest {
            id: question.id,
            execution: self.request.authority.identity.execution.clone(),
            expected_revision: self.revision.clone(),
            question: question.question,
            answer_kind: question.answer_kind,
            required_authority: question.required_authority,
        };
        self.published = Some(request.clone());
        if !already {
            self.retain_snapshot()?;
        }
        Ok(DriveResult::NeedDirection { request })
    }

    fn accept_direction(&mut self, reply: DirectionReply) -> Result<DriveResult, ServiceError> {
        let answer = digest(&[reply.answer.as_bytes()]);
        self.append_direction(&reply.decision, &answer)?;
        self.consumed = Some((reply.decision, reply.answer));
        self.consumed_answer = Some(answer);
        self.bump();
        self.retain_snapshot()?;
        Ok(self.advanced())
    }

    fn note_direction(&mut self, reply: DirectionReply) -> DriveResult {
        self.consumed_answer = Some(digest(&[reply.answer.as_bytes()]));
        self.consumed = Some((reply.decision, reply.answer));
        DriveResult::Quiescent { wake_on: wake_on() }
    }

    fn hold(&mut self, mode: Mode, label: &str) -> Result<DriveResult, ServiceError> {
        let marker = digest(&[label.as_bytes()]);
        self.append_cause(label, "session-mode", &marker)?;
        self.mode = mode;
        self.bump();
        self.republish_fence();
        self.retain_snapshot()?;
        Ok(self.advanced())
    }

    fn stop_one(&mut self) -> Result<DriveResult, ServiceError> {
        let marker = digest(&[b"stopped"]);
        self.append_cause("control-stop", "session-mode", &marker)?;
        self.mode = Mode::Stopped;
        self.bump();
        self.republish_fence();
        self.retain_snapshot()?;
        Ok(self.advanced())
    }

    fn republish_fence(&mut self) {
        if self.consumed.is_none()
            && let Some(request) = self.published.as_mut()
        {
            request.expected_revision = self.revision.clone();
        }
    }

    fn begin_effect(&mut self, inputs: &InputBatch) -> Result<Prepared, ServiceError> {
        if self.pending_effect.is_some() {
            return Err(ServiceError::EffectOutstanding);
        }
        if self.missing_begin() {
            return Err(ServiceError::MissingBegin);
        }
        if self
            .cancellation
            .cancellation_requested(&self.request.authority.identity.execution)
        {
            let result = self.stop_one()?;
            self.last_drive = result.clone();
            return Ok(done(result));
        }
        let Some(leaf) = self
            .readiness
            .admitted_leaf(&self.request.authority.identity.roadmap)?
        else {
            let result = DriveResult::Quiescent { wake_on: wake_on() };
            self.last_drive = result.clone();
            return Ok(done(result));
        };
        if leaf != self.request.phase {
            return Err(ServiceError::WrongLeaf);
        }
        if self.transition.state == PhaseRunState::Succeeded
            || self.transition.state == PhaseRunState::Failed
            || self.transition.state == PhaseRunState::Cancelled
        {
            let result = DriveResult::Complete {
                seal_input: self.revision.clone(),
            };
            self.last_drive = result.clone();
            return Ok(done(result));
        }
        // A phase copied from the journal is not a reducer state.
        if !self.transition_faithful {
            return Err(ServiceError::RestoredTransition);
        }
        let before = self.journal.rebuild()?;
        let effect_id = format!("{}:{}", leaf, before.records.len());
        let kind = inputs
            .product
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "admitted-leaf".to_owned());
        let intent_head = self.journal.append(
            Some(&before.head),
            ExecutionRecord::EffectRequested {
                effect_id: effect_id.clone(),
                kind,
                input: None,
            },
            vec![],
        )?;
        self.pending_effect = Some(effect_id.clone());
        Ok(Prepared::Ready(Box::new(EffectTicket {
            identity: self.request.authority.identity.clone(),
            leaf,
            transition: self.transition.clone(),
            effect_id,
            intent_head,
            product: inputs.product.clone(),
        })))
    }

    fn remember_offer(&mut self, inputs: &InputBatch) {
        for wake in &inputs.wakes {
            if let WakeEvent::ResourceOffer { kind, amount } = wake {
                self.seen_offers.insert((kind.clone(), *amount));
            }
        }
    }
}
