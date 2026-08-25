use std::sync::Arc;

use sim_kernel::ContentId;
use sim_lib_journal::JournalBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{
    AttemptId, ExecutionEvent, ExecutionFailure, ExecutionPolicy, MutationPlan, PhaseRunState,
    Transition, reduce, replay,
};
use thiserror::Error;

use crate::{
    AuthorityGrant, ExecutionJournal, ExecutionJournalError, ExecutionPins, ExecutionRecord,
    Limits, RebuiltExecution,
};

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub journal: RebuiltExecution,
    pub transition: Transition,
}

pub struct RoadmapRunnerService<B, R, E, C> {
    journal: ExecutionJournal<B>,
    readiness: R,
    effects: E,
    cancellation: C,
    request: OpenRequest,
    transition: Transition,
    opening_grant: ContentId,
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
        journal.open(pins(&request), None)?;
        let opening_grant = request.authority.grant.clone();
        Ok(Self {
            journal,
            readiness,
            effects,
            cancellation,
            request,
            transition: Transition::default(),
            opening_grant,
        })
    }

    /// Performs at most one admitted leaf effect. There is no hidden run loop.
    pub fn advance_one_effect(&mut self) -> Result<Inspection, ServiceError> {
        if self
            .cancellation
            .cancellation_requested(&self.request.authority.identity.execution)
        {
            return self.cancel();
        }
        let Some(leaf) = self
            .readiness
            .admitted_leaf(&self.request.authority.identity.roadmap)?
        else {
            return Err(ServiceError::NotReady);
        };
        if leaf != self.request.phase {
            return Err(ServiceError::WrongLeaf);
        }
        let before = self.journal.rebuild()?;
        let effect_id = format!("{}:{}", leaf, before.records.len());
        let intent_head = self.journal.append(
            Some(&before.head),
            ExecutionRecord::EffectRequested {
                effect_id: effect_id.clone(),
                kind: "admitted-leaf".into(),
                input: None,
            },
            vec![],
        )?;
        let event =
            self.effects
                .invoke(&self.request.authority.identity, &leaf, &self.transition)?;
        let next = reduce(
            &self.request.policy,
            &self.request.mutation,
            &self.request.authority.identity.execution,
            &leaf,
            &self.request.attempt,
            &self.transition,
            &event,
        )?;
        let receipt_head = self.journal.append(
            Some(&intent_head),
            ExecutionRecord::EffectReceipt {
                effect_id,
                outcome: event.observation.kind.to_string(),
                output: None,
            },
            vec![],
        )?;
        self.journal.append(
            Some(&receipt_head),
            ExecutionRecord::StateTransition {
                from: state_text(self.transition.state).into(),
                to: state_text(next.state).into(),
            },
            vec![],
        )?;
        self.transition = next;
        self.inspect()
    }

    pub fn inspect(&self) -> Result<Inspection, ServiceError> {
        Ok(Inspection {
            journal: self.journal.rebuild()?,
            transition: self.transition.clone(),
        })
    }

    pub fn replay(&self) -> Result<Transition, ServiceError> {
        let events = self.effects.receipts(&self.request.authority.identity)?;
        Ok(replay(
            &self.request.policy,
            &self.request.mutation,
            &self.request.authority.identity.execution,
            &self.request.phase,
            &self.request.attempt,
            &Transition::default(),
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
        self.transition = self.replay()?;
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

    fn cancel(&mut self) -> Result<Inspection, ServiceError> {
        let state = self.journal.rebuild()?;
        let event = ExecutionEvent {
            execution: self.request.authority.identity.execution.clone(),
            phase: self.request.phase.clone(),
            attempt: self.request.attempt.clone(),
            observation: sim_roadmap_exec_core::Observation {
                kind: sim_kernel::Symbol::new("cancel"),
                journal_head: state.head.entry.clone(),
                ..Default::default()
            },
        };
        let next = self.propose_transition(&event)?;
        self.journal.append(
            Some(&state.head),
            ExecutionRecord::StateTransition {
                from: state_text(self.transition.state).into(),
                to: state_text(PhaseRunState::Cancelled).into(),
            },
            vec![],
        )?;
        self.transition = next;
        self.inspect()
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
}
