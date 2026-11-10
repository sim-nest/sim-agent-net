// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;
use std::sync::Arc;

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::JournalBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{
    AttemptId, ExecutionEvent, ExecutionFailure, ExecutionPolicy, MutationPlan, PhaseRunState,
    Transition, reduce, replay,
};
use sim_work_core::ResourceNeed;
use thiserror::Error;

use crate::{
    AuthorityGrant, ExecutionJournal, ExecutionJournalError, ExecutionPins, ExecutionRecord, Lease,
    Limits, Mode, RebuiltExecution,
};

#[path = "drive.rs"]
mod drive;
#[path = "drive_commit.rs"]
mod drive_commit;
#[path = "drive_step.rs"]
mod drive_step;

pub use drive::{
    DirectionHandoff, DirectionKind, DirectionQuestion, DirectionReply, DirectionRequest,
    DriveResult, EffectTicket, ExternalLineageRef, InputBatch, LineageView, Prepared, RunSnapshot,
    SnapshotRead, SnapshotRow, direction_shape,
};
pub use sim_work_core::CapabilityGrant;

/// Port over the compiled plan. The service intentionally owns no dependency
/// graph or ready queue.
pub trait ReadinessPort {
    fn admitted_leaf(&self, roadmap: &ContentId) -> Result<Option<PhaseId>, ServiceError>;
}

/// Bound production/fake adapters. An adapter returns an observation; only the
/// pure reducer is allowed to turn that observation into state.
pub trait EffectPort {
    fn invoke(
        &self,
        identity: &crate::ExecutionIdentity,
        phase: &PhaseId,
        current: &Transition,
        product: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError>;
    fn receipts(
        &self,
        identity: &crate::ExecutionIdentity,
    ) -> Result<Vec<ExecutionEvent>, ServiceError>;
}

pub trait CancellationPort {
    fn cancellation_requested(&self, execution: &sim_roadmap_exec_core::ExecutionId) -> bool;
}

#[derive(Clone, Debug)]
pub struct OpenRequest {
    pub authority: AuthorityGrant,
    pub phase: PhaseId,
    pub attempt: AttemptId,
    pub policy: ExecutionPolicy,
    pub mutation: MutationPlan,
    /// Resources that must be offered before an effect starts.
    pub resources: Vec<ResourceNeed>,
    /// Direction question published before an effect, when one is bound.
    pub direction: Option<DirectionQuestion>,
    /// Selection handoff. Begin authority stays separate from selection.
    pub handoff: Option<DirectionHandoff>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub journal: RebuiltExecution,
    pub transition: Transition,
    /// Last conduct result. Reading it performs no effect.
    pub drive: DriveResult,
}

pub struct RoadmapRunnerService<B, R, E, C> {
    journal: ExecutionJournal<B>,
    readiness: R,
    effects: Arc<E>,
    cancellation: C,
    request: OpenRequest,
    transition: Transition,
    /// Reducer fields match durable observations. A copied phase alone does not.
    transition_faithful: bool,
    opening_head: ContentId,
    opening_grant: ContentId,
    revision_n: u64,
    revision: ContentId,
    last_drive: DriveResult,
    uncertain: bool,
    mode: Mode,
    resources: Vec<ResourceNeed>,
    question: Option<DirectionQuestion>,
    published: Option<DirectionRequest>,
    consumed: Option<(ContentId, String)>,
    consumed_answer: Option<ContentId>,
    wait_resumed: bool,
    seen_offers: BTreeSet<(String, u64)>,
    handoff: Option<DirectionHandoff>,
    snapshots: Vec<RunSnapshot>,
    leases: Vec<Lease>,
    /// Effect admitted by `prepare_effect` and not yet committed.
    pending_effect: Option<String>,
}

impl<B: JournalBackend, R: ReadinessPort, E: EffectPort, C: CancellationPort>
    RoadmapRunnerService<B, R, E, C>
{
    pub fn open(
        backend: Arc<B>,
        readiness: R,
        effects: E,
        cancellation: C,
        request: OpenRequest,
        limits: Limits,
    ) -> Result<Self, ServiceError> {
        validate_request(&request)?;
        let execution_text = request.authority.identity.execution.to_string();
        let journal = ExecutionJournal::new(backend, execution_text, limits);
        let opened = journal.open(pins(&request), None)?;
        let opening_head = opened.opening_head.entry;
        let opening_grant = request.authority.grant.clone();
        let resources = request.resources.clone();
        let question = request.direction.clone();
        let handoff = request.handoff.clone();
        let revision = drive::revision_id(&opening_head, 0);
        let mut service = Self {
            journal,
            readiness,
            effects: Arc::new(effects),
            cancellation,
            request,
            transition: Transition::planned(opening_head.clone()),
            transition_faithful: true,
            opening_head,
            opening_grant,
            revision_n: 0,
            revision,
            last_drive: DriveResult::Quiescent {
                wake_on: drive::wake_on(),
            },
            uncertain: false,
            mode: Mode::Running,
            resources,
            question,
            published: None,
            consumed: None,
            consumed_answer: None,
            wait_resumed: false,
            seen_offers: BTreeSet::new(),
            handoff,
            snapshots: Vec::new(),
            leases: Vec::new(),
            pending_effect: None,
        };
        if service.journal_uncertain()? {
            service.uncertain = true;
        }
        service.restore_execution()?;
        service.restore_direction()?;
        Ok(service)
    }

    /// Clone of the effect port. Invoke it with no borrow of this service.
    pub fn effect_port(&self) -> Arc<E> {
        Arc::clone(&self.effects)
    }

    /// An admitted effect has not committed. Reopen must leave that ticket in place.
    pub fn effect_outstanding(&self) -> bool {
        self.pending_effect.is_some()
    }

    pub fn inspect(&self) -> Result<Inspection, ServiceError> {
        Ok(Inspection {
            journal: self.journal.rebuild()?,
            transition: self.transition.clone(),
            drive: self.last_drive.clone(),
        })
    }

    pub fn replay(&self) -> Result<Transition, ServiceError> {
        if self.effect_outstanding() {
            return Err(ServiceError::EffectOutstanding);
        }
        let events = self.effects.receipts(&self.request.authority.identity)?;
        Ok(replay(
            &self.request.policy,
            &self.request.mutation,
            &self.request.authority.identity.execution,
            &self.request.phase,
            &self.request.attempt,
            &Transition::planned(self.opening_head.clone()),
            &events,
        )?)
    }

    pub fn resume(&mut self, authority: AuthorityGrant) -> Result<Inspection, ServiceError> {
        if authority.identity != self.request.authority.identity {
            return Err(ServiceError::IdentityDrift);
        }
        if authority.grant == self.opening_grant {
            return Err(ServiceError::FreshAuthorityRequired);
        }
        if !authority
            .ceiling
            .is_narrower_than(&self.request.authority.ceiling)
        {
            return Err(ServiceError::BudgetWidening);
        }
        self.request.authority = authority;
        self.inspect()
    }

    pub fn propose_transition(&self, event: &ExecutionEvent) -> Result<Transition, ServiceError> {
        Ok(reduce(
            &self.request.policy,
            &self.request.mutation,
            &self.request.authority.identity.execution,
            &self.request.phase,
            &self.request.attempt,
            &self.transition,
            event,
        )?)
    }
}

fn validate_request(request: &OpenRequest) -> Result<(), ServiceError> {
    let id = &request.authority.identity;
    if id.policy != request.policy.id || id.source_deck != request.policy.source_deck {
        return Err(ServiceError::IdentityDrift);
    }
    Ok(())
}
fn pins(request: &OpenRequest) -> ExecutionPins {
    let id = &request.authority.identity;
    ExecutionPins {
        conduct: content_text(&id.conduct),
        policy: id.policy.to_string(),
        source_deck: id.source_deck.clone(),
        model_pick: content_text(&id.model),
        runner_generation: content_text(&id.runner),
    }
}

fn content_text(id: &ContentId) -> String {
    let mut text = format!("{}:", id.algorithm);
    for byte in id.bytes {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}

fn state_text(state: PhaseRunState) -> &'static str {
    match state {
        PhaseRunState::Planned => "planned",
        PhaseRunState::Running => "running",
        PhaseRunState::Reconciling => "reconciling",
        PhaseRunState::Succeeded => "succeeded",
        PhaseRunState::Failed => "failed",
        PhaseRunState::Cancelled => "cancelled",
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Journal(#[from] ExecutionJournalError),
    #[error("pure reducer rejected adapter receipt: {0}")]
    Reducer(#[from] ExecutionFailure),
    #[error("compiled plan has no admitted leaf")]
    NotReady,
    #[error("compiled plan admitted a different leaf")]
    WrongLeaf,
    #[error("a pinned execution identity changed")]
    IdentityDrift,
    #[error("resume requires fresh caller authority")]
    FreshAuthorityRequired,
    #[error("resume attempted to widen the effective ceiling")]
    BudgetWidening,
    #[error("adapter failed: {0}")]
    Adapter(String),
    #[error("direction revision is stale")]
    StaleDirection,
    #[error("direction reply does not match the published request")]
    WrongDirection,
    #[error("direction decision was already consumed")]
    ReplayedDirection,
    #[error("advisory input cannot authorize a decision")]
    AdvisoryDirection,
    #[error("the grant does not include the required direction authority")]
    DirectionAuthority,
    #[error("selection supplies no begin authority")]
    MissingBegin,
    #[error("the effect outcome is uncertain and is not invoked again")]
    EffectUncertain,
    #[error("the restored execution has no reducer state")]
    RestoredTransition,
    #[error("an admitted effect is still outstanding or the ticket does not match")]
    EffectOutstanding,
}
