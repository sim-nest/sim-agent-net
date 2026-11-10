// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one advance performs one effect or one pure transition.

use super::drive_harness::advance_one_effect;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{
    AttemptId, ExecutionEvent, ExecutionId, ExecutionPolicy, ExecutionPolicyId, MutationId,
    MutationPlan, Observation, PhaseRunState,
};
use sim_work_core::ResourceNeed;

use crate::{
    AuthorityGrant, CapabilityGrant, ControlIntent, DirectionKind, DirectionQuestion,
    DirectionReply, DriveResult, EffectPort, ExecutionIdentity, InputBatch, Lease, Limits,
    OpenRequest, ReadinessPort, RoadmapRunnerService, ServiceError, SnapshotRead, WakeCause,
    WakeEvent,
};

pub(crate) fn cid(n: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32])
}

pub(crate) fn request() -> OpenRequest {
    let policy = ExecutionPolicy {
        id: ExecutionPolicyId::new("policy").unwrap(),
        source_deck: cid(2),
        required_promises: vec![],
        required_proofs: vec![],
    };
    OpenRequest {
        authority: AuthorityGrant {
            identity: ExecutionIdentity {
                execution: ExecutionId::new("execution").unwrap(),
                policy: policy.id.clone(),
                roadmap: cid(1),
                source_deck: policy.source_deck.clone(),
                conduct: cid(3),
                model: cid(4),
                launcher: cid(5),
                runner: cid(6),
            },
            ceiling: crate::EffectiveCeiling::intersect([crate::OwnedLimit {
                owner: Symbol::new("caller"),
                unit: Symbol::new("turns"),
                amount: 4,
            }]),
            grant: cid(7),
        },
        phase: PhaseId::new("leaf").unwrap(),
        attempt: AttemptId::new("attempt").unwrap(),
        policy,
        mutation: MutationPlan::new(MutationId::new("mutation").unwrap(), vec![], vec![]).unwrap(),
        resources: vec![],
        direction: None,
        handoff: None,
    }
}

pub(crate) struct Ready(pub(crate) PhaseId);
impl ReadinessPort for Ready {
    fn admitted_leaf(&self, _: &ContentId) -> Result<Option<PhaseId>, ServiceError> {
        Ok(Some(self.0.clone()))
    }
}

pub(crate) struct CountEffects(pub(crate) Arc<AtomicUsize>);
impl EffectPort for CountEffects {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(Symbol::new("start"), cid(20)),
        })
    }
    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

pub(crate) struct FailEffects(Arc<AtomicUsize>);
impl EffectPort for FailEffects {
    fn invoke(
        &self,
        _: &ExecutionIdentity,
        _: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ServiceError::Adapter("unavailable".into()))
    }
    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

pub(crate) struct Never;
impl crate::CancellationPort for Never {
    fn cancellation_requested(&self, _: &ExecutionId) -> bool {
        false
    }
}

pub(crate) fn open_count(
    request: OpenRequest,
    calls: Arc<AtomicUsize>,
) -> RoadmapRunnerService<MemoryBackend, Ready, CountEffects, Never> {
    let phase = request.phase.clone();
    RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(phase),
        CountEffects(calls),
        Never,
        request,
        Limits::default(),
    )
    .unwrap()
}

pub(crate) fn open_fail(
    request: OpenRequest,
    calls: Arc<AtomicUsize>,
) -> RoadmapRunnerService<MemoryBackend, Ready, FailEffects, Never> {
    open_fail_on(Arc::new(MemoryBackend::new()), request, calls)
}

pub(crate) fn open_fail_on(
    backend: Arc<MemoryBackend>,
    request: OpenRequest,
    calls: Arc<AtomicUsize>,
) -> RoadmapRunnerService<MemoryBackend, Ready, FailEffects, Never> {
    let phase = request.phase.clone();
    RoadmapRunnerService::open(
        backend,
        Ready(phase),
        FailEffects(calls),
        Never,
        request,
        Limits::default(),
    )
    .unwrap()
}

pub(crate) struct RejectEffects(Arc<AtomicUsize>);
impl EffectPort for RejectEffects {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(Symbol::new("nope"), cid(21)),
        })
    }
    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

pub(crate) fn open_reject(
    backend: Arc<MemoryBackend>,
    request: OpenRequest,
    calls: Arc<AtomicUsize>,
) -> RoadmapRunnerService<MemoryBackend, Ready, RejectEffects, Never> {
    let phase = request.phase.clone();
    RoadmapRunnerService::open(
        backend,
        Ready(phase),
        RejectEffects(calls),
        Never,
        request,
        Limits::default(),
    )
    .unwrap()
}

pub(crate) fn question(authority: Option<Symbol>) -> DirectionQuestion {
    DirectionQuestion {
        id: cid(30),
        question: "continue".into(),
        answer_kind: DirectionKind::Decision,
        required_authority: authority,
    }
}

pub(crate) fn reply(
    request: &crate::DirectionRequest,
    kind: DirectionKind,
    answer: &str,
) -> DirectionReply {
    DirectionReply {
        request: request.id.clone(),
        execution: request.execution.clone(),
        expected_revision: request.expected_revision.clone(),
        answer_kind: kind,
        answer: answer.into(),
        decision: cid(40),
    }
}

pub(crate) fn grant(name: &str) -> CapabilityGrant {
    CapabilityGrant {
        capabilities: vec![Symbol::new(name)],
    }
}

#[test]
fn no_change_wake_starts_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(request(), Arc::clone(&calls));
    let before = service.inspect().unwrap().journal.records.len();
    let driven = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(driven, DriveResult::Quiescent { .. }));
    assert_eq!(service.inspect().unwrap().journal.records.len(), before);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Planned
    );
}

#[test]
fn resource_wait_resumes_once_on_a_material_offer() {
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let blocked = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(blocked, DriveResult::Blocked { .. }));
    assert!(service.leases().is_empty());
    let quiet = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(
        quiet,
        DriveResult::Quiescent { wake_on } if wake_on.contains(&WakeCause::ResourceOffer)
    ));
    let mut zero = InputBatch::no_change();
    zero.wakes.push(WakeEvent::ResourceOffer {
        kind: "slot".into(),
        amount: 0,
    });
    advance_one_effect(&mut service, &zero, &CapabilityGrant::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut offer = InputBatch::no_change();
    offer.wakes.push(WakeEvent::ResourceOffer {
        kind: "slot".into(),
        amount: 2,
    });
    let resumed = advance_one_effect(&mut service, &offer, &CapabilityGrant::default()).unwrap();
    assert!(matches!(resumed, DriveResult::Advanced { .. }));
    advance_one_effect(&mut service, &offer, &CapabilityGrant::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(service.leases().is_empty());
    let _: &[Lease] = service.leases();
}

#[test]
fn direction_refuses_stale_wrong_replayed_and_advisory_answers() {
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let published =
        advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap();
    let DriveResult::NeedDirection { request } = published else {
        panic!("direction was not published");
    };
    let published_len = service.inspect().unwrap().journal.records.len();
    advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap();
    assert_eq!(
        service.inspect().unwrap().journal.records.len(),
        published_len
    );
    let mut wrong = reply(&request, DirectionKind::Decision, "yes");
    wrong.execution = ExecutionId::new("other").unwrap();
    let mut batch = InputBatch::no_change();
    batch.reply = Some(wrong);
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &grant("decide")),
        Err(ServiceError::WrongDirection)
    ));
    batch.reply = Some(reply(&request, DirectionKind::Advice, "maybe"));
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &grant("decide")),
        Err(ServiceError::AdvisoryDirection)
    ));
    let mut stale = reply(&request, DirectionKind::Decision, "yes");
    stale.expected_revision = cid(99);
    batch.reply = Some(stale);
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &grant("decide")),
        Err(ServiceError::StaleDirection)
    ));
    assert_eq!(
        service.inspect().unwrap().journal.records.len(),
        published_len
    );
    batch.reply = Some(reply(&request, DirectionKind::Decision, "yes"));
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &CapabilityGrant::default()),
        Err(ServiceError::DirectionAuthority)
    ));
    let accepted = advance_one_effect(&mut service, &batch, &grant("decide")).unwrap();
    assert!(matches!(accepted, DriveResult::Advanced { .. }));
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Planned
    );
    let after = service.inspect().unwrap().journal.records.len();
    let replayed = advance_one_effect(&mut service, &batch, &grant("decide")).unwrap();
    assert!(matches!(replayed, DriveResult::Quiescent { .. }));
    assert_eq!(service.inspect().unwrap().journal.records.len(), after);
    batch.reply = Some(reply(&request, DirectionKind::Decision, "no"));
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &grant("decide")),
        Err(ServiceError::ReplayedDirection)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let ran = advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap();
    assert!(matches!(ran, DriveResult::Advanced { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
}

#[test]
fn informational_direction_does_not_change_state() {
    let mut open_request = request();
    open_request.direction = Some(question(None));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let DriveResult::NeedDirection { request } = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap() else {
        panic!("direction was not published");
    };
    let len = service.inspect().unwrap().journal.records.len();
    let mut batch = InputBatch::conduct();
    batch.reply = Some(reply(&request, DirectionKind::Decision, "note"));
    let again = advance_one_effect(&mut service, &batch, &CapabilityGrant::default()).unwrap();
    assert!(matches!(again, DriveResult::Quiescent { .. }));
    assert_eq!(service.inspect().unwrap().journal.records.len(), len);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Planned
    );
    let ran = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(ran, DriveResult::Advanced { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
}

#[test]
fn snapshot_pages_stay_on_the_retained_revision() {
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let DriveResult::NeedDirection { request } =
        advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap()
    else {
        panic!("direction was not published");
    };
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&request, DirectionKind::Decision, "yes"));
    advance_one_effect(&mut service, &batch, &grant("decide")).unwrap();
    let first = service.snapshots().last().unwrap().clone();
    assert!(first.rows.len() > 1);
    assert!(
        first
            .rows
            .iter()
            .any(|row| row.key == "direction-request" && row.value == cid(30))
    );
    assert!(
        first
            .rows
            .iter()
            .any(|row| row.key == "direction-decision" && row.value == cid(40))
    );
    let journal_state = service
        .inspect()
        .unwrap()
        .journal
        .snapshots
        .last()
        .unwrap()
        .state
        .clone();
    assert_eq!(first.id, journal_state);
    let page = service.read_snapshot(&first.id, 0, 1, None);
    let SnapshotRead::Page {
        revision,
        rows,
        continuation: Some(next),
        ..
    } = page
    else {
        panic!("first page did not continue");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(revision, first.revision);
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    assert_ne!(service.revision(), &first.revision);
    let retained = service.read_snapshot(&first.id, next, 1, None);
    let SnapshotRead::Page {
        revision: page_revision,
        rows: page_rows,
        ..
    } = retained
    else {
        panic!("continuation left the retained snapshot");
    };
    assert_eq!(page_revision, first.revision);
    assert_eq!(page_rows, vec![first.rows[next as usize].clone()]);
    assert!(page_rows.iter().all(|row| row.key != "session-mode"));
    let stale = service.read_snapshot(&first.id, next, 1, Some(service.revision()));
    assert!(matches!(stale, SnapshotRead::Stale { .. }));
    assert!(matches!(
        service.read_snapshot(&cid(77), 0, 1, None),
        SnapshotRead::Missing
    ));
    let len = service.inspect().unwrap().journal.records.len();
    let _ = service.read_snapshot(&first.id, 0, 1, None);
    assert_eq!(service.inspect().unwrap().journal.records.len(), len);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
