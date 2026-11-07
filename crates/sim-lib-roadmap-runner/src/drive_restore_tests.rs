// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one advance performs one effect or one pure transition.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sim_kernel::{Datum, Symbol};
use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{AttemptId, ExecutionEvent, ExecutionId, Observation, PhaseRunState};
use sim_work_core::ResourceNeed;

use super::drive_tests::{CountEffects, Never, Ready, grant, question, reply, request};
use crate::{
    CancellationPort, CapabilityGrant, ControlIntent, DirectionKind, DriveResult, EffectPort,
    ExecutionIdentity, ExecutionRecord, InputBatch, Limits, Mode, OpenRequest, OwnedLimit,
    ReadinessPort, RoadmapRunnerService, ServiceError, WakeEvent,
};

fn record_len<R, E, C>(service: &RoadmapRunnerService<MemoryBackend, R, E, C>) -> usize
where
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    service.inspect().unwrap().journal.records.len()
}

fn requested<R, E, C>(service: &RoadmapRunnerService<MemoryBackend, R, E, C>) -> usize
where
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    service
        .inspect()
        .unwrap()
        .journal
        .records
        .iter()
        .filter(|record| matches!(record, ExecutionRecord::EffectRequested { .. }))
        .count()
}

fn stop_count<R, E, C>(service: &RoadmapRunnerService<MemoryBackend, R, E, C>) -> usize
where
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    service
        .inspect()
        .unwrap()
        .journal
        .records
        .iter()
        .filter(|record| {
            matches!(record, ExecutionRecord::SemanticDelta { cause, .. } if cause == "control-stop")
        })
        .count()
}

fn uncertain_marker<R, E, C>(service: &RoadmapRunnerService<MemoryBackend, R, E, C>) -> bool
where
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    service.inspect().unwrap().journal.records.iter().any(|record| {
        matches!(record, ExecutionRecord::SemanticDelta { cause, .. } if cause == "effect-uncertain")
    })
}

struct Remembered {
    calls: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<ExecutionEvent>>>,
}

impl EffectPort for Remembered {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let event = ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(Symbol::new("start"), super::drive_tests::cid(20)),
        };
        self.events.lock().unwrap().push(event.clone());
        Ok(event)
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

struct CancelGate(Arc<AtomicBool>);

impl CancellationPort for CancelGate {
    fn cancellation_requested(&self, _: &ExecutionId) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn narrower(open_request: &OpenRequest) -> crate::AuthorityGrant {
    let mut authority = open_request.authority.clone();
    authority.grant = super::drive_tests::cid(8);
    authority.ceiling = crate::EffectiveCeiling::intersect([OwnedLimit {
        owner: Symbol::new("caller"),
        unit: Symbol::new("turns"),
        amount: 3,
    }]);
    authority
}

#[test]
fn empty_receipts_refuse_the_next_effect_without_a_new_request() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = crate::RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    let kept = record_len(&service);
    let requested_before = requested(&service);
    let mut reopened = crate::RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        reopened.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    assert!(matches!(
        reopened.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Err(ServiceError::RestoredTransition)
    ));
    assert!(matches!(
        reopened.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Err(ServiceError::RestoredTransition)
    ));
    assert_eq!(record_len(&reopened), kept);
    assert_eq!(requested(&reopened), requested_before);
    assert!(!uncertain_marker(&reopened));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn recorded_receipts_rebuild_the_reducer_on_open() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let open_request = request();
    let phase = open_request.phase.clone();
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(phase.clone()),
        Remembered {
            calls: Arc::clone(&calls),
            events: Arc::clone(&events),
        },
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    let reopened = RoadmapRunnerService::open(
        backend,
        Ready(phase),
        Remembered {
            calls: Arc::clone(&calls),
            events,
        },
        Never,
        open_request,
        Limits::default(),
    )
    .unwrap();
    let transition = reopened.inspect().unwrap().transition;
    assert_eq!(transition.state, PhaseRunState::Running);
    assert!(!transition.requested_effects.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn resume_keeps_the_restored_phase() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    let mut reopened = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    let seen = reopened.resume(narrower(&open_request)).unwrap();
    assert_eq!(seen.transition.state, PhaseRunState::Running);
    assert_eq!(
        reopened.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn reopened_direction_answer_stays_quiescent() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    let DriveResult::NeedDirection { request: published } = service
        .advance_one_effect(&InputBatch::conduct(), &grant("decide"))
        .unwrap()
    else {
        panic!("direction was not published");
    };
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&published, DirectionKind::Decision, "yes"));
    service
        .advance_one_effect(&batch, &grant("decide"))
        .unwrap();
    let after = record_len(&service);
    let revision = service.revision().clone();
    let mut reopened = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(reopened.revision(), &revision);
    let same = reopened
        .advance_one_effect(&batch, &grant("decide"))
        .unwrap();
    assert!(matches!(same, DriveResult::Quiescent { .. }));
    assert_eq!(record_len(&reopened), after);
    batch.reply = Some(reply(&published, DirectionKind::Decision, "no"));
    assert!(matches!(
        reopened.advance_one_effect(&batch, &grant("decide")),
        Err(ServiceError::ReplayedDirection)
    ));
    batch.reply = Some(reply(&published, DirectionKind::Decision, ""));
    assert!(matches!(
        reopened.advance_one_effect(&batch, &grant("decide")),
        Err(ServiceError::ReplayedDirection)
    ));
    assert_eq!(record_len(&reopened), after);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

fn assert_armed_stop(
    service: &mut RoadmapRunnerService<MemoryBackend, Ready, CountEffects, CancelGate>,
    calls: &AtomicUsize,
) {
    let before = record_len(service);
    let driven = service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(driven, DriveResult::Advanced { .. }));
    assert_eq!(service.mode(), Mode::Stopped);
    assert_eq!(stop_count(service), 1);
    assert_eq!(requested(service), 0);
    assert!(record_len(service) > before);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let after = record_len(service);
    let again = service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(again, DriveResult::Quiescent { .. }));
    assert_eq!(record_len(service), after);
    assert_eq!(service.mode(), Mode::Stopped);
}

#[test]
fn cancellation_stops_a_direction_wait() {
    let armed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        CancelGate(Arc::clone(&armed)),
        open_request,
        Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        service.advance_one_effect(&InputBatch::conduct(), &grant("decide")),
        Ok(DriveResult::NeedDirection { .. })
    ));
    armed.store(true, Ordering::SeqCst);
    assert_armed_stop(&mut service, &calls);
}

#[test]
fn cancellation_stops_a_resource_wait() {
    let armed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        CancelGate(Arc::clone(&armed)),
        open_request,
        Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        service.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Ok(DriveResult::Blocked { .. })
    ));
    armed.store(true, Ordering::SeqCst);
    assert_armed_stop(&mut service, &calls);
}

#[test]
fn cancellation_stops_while_paused() {
    let armed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        CancelGate(Arc::clone(&armed)),
        open_request,
        Limits::default(),
    )
    .unwrap();
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    service
        .advance_one_effect(&pause, &CapabilityGrant::default())
        .unwrap();
    assert_eq!(service.mode(), Mode::Paused);
    armed.store(true, Ordering::SeqCst);
    assert_armed_stop(&mut service, &calls);
}
