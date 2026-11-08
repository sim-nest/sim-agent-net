// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one advance performs one effect or one pure transition.

use super::drive_harness::advance_one_effect;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sim_kernel::{Datum, Symbol};
use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{AttemptId, ExecutionEvent, Observation, PhaseRunState};
use sim_work_core::ResourceNeed;

use super::drive_tests::{CountEffects, Never, Ready, grant, question, reply, request};
use crate::{
    CapabilityGrant, ControlIntent, DirectionHandoff, DirectionKind, DriveResult, EffectPort,
    ExecutionIdentity, ExecutionJournal, ExecutionRecord, InputBatch, Limits, RoadmapRunnerService,
    ServiceError, SnapshotRead, WakeEvent,
};

fn records(service: &RoadmapRunnerService<MemoryBackend, Ready, CountEffects, Never>) -> usize {
    service.inspect().unwrap().journal.records.len()
}

fn open_on(
    backend: Arc<MemoryBackend>,
    open_request: crate::OpenRequest,
    calls: Arc<AtomicUsize>,
) -> RoadmapRunnerService<MemoryBackend, Ready, CountEffects, Never> {
    let phase = open_request.phase.clone();
    RoadmapRunnerService::open(
        backend,
        Ready(phase),
        CountEffects(calls),
        Never,
        open_request,
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn no_change_does_not_publish_a_direction_request() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let mut service = open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    let before = records(&service);
    let quiet = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(quiet, DriveResult::Quiescent { .. }));
    assert_eq!(records(&service), before);
    let published = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(published, DriveResult::NeedDirection { .. }));
    let after = records(&service);
    assert!(after > before);
    let again = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(again, DriveResult::NeedDirection { .. }));
    assert_eq!(records(&service), after);
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    let restored = advance_one_effect(
        &mut reopened,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(restored, DriveResult::NeedDirection { .. }));
    assert_eq!(records(&reopened), after);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_begin_refuses_before_a_direction_is_published() {
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    open_request.handoff = Some(DirectionHandoff {
        exact_source: super::drive_tests::cid(11),
        candidate: super::drive_tests::cid(12),
        selection: super::drive_tests::cid(13),
        begin: None,
        external_lineage: vec![],
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_on(
        Arc::new(MemoryBackend::new()),
        open_request,
        Arc::clone(&calls),
    );
    let before = records(&service);
    let quiet = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(quiet, DriveResult::Quiescent { .. }));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::MissingBegin)
    ));
    let mut offer = InputBatch::no_change();
    offer.wakes.push(WakeEvent::ResourceOffer {
        kind: "unused".into(),
        amount: 1,
    });
    assert!(matches!(
        advance_one_effect(&mut service, &offer, &CapabilityGrant::default()),
        Err(ServiceError::MissingBegin)
    ));
    assert_eq!(records(&service), before);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.handoff().unwrap().selection,
        super::drive_tests::cid(13)
    );
}

#[test]
fn paused_wait_conduct_stays_quiescent() {
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_on(
        Arc::new(MemoryBackend::new()),
        open_request,
        Arc::clone(&calls),
    );
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    let driven = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(driven, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(service.leases().is_empty());
}

#[test]
fn reopen_keeps_the_committed_phase() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    let saved = service.snapshots().to_vec();
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    assert_eq!(
        reopened.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    assert_eq!(reopened.snapshots(), saved.as_slice());
    let quiet = advance_one_effect(
        &mut reopened,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(quiet, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        reopened.read_snapshot(&saved[0].id, 0, 4, Some(&saved[0].revision)),
        SnapshotRead::Page { .. }
    ));
}

#[test]
fn paused_wait_stays_quiescent_after_reopen() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let mut service = open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    let driven = advance_one_effect(
        &mut reopened,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert!(matches!(driven, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn reopened_pause_keeps_the_republished_fence() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let mut service = open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    let DriveResult::NeedDirection { request: published } =
        advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap()
    else {
        panic!("direction was not published");
    };
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    let mut stale = InputBatch::no_change();
    stale.reply = Some(reply(&published, DirectionKind::Decision, "yes"));
    assert!(matches!(
        advance_one_effect(&mut reopened, &stale, &grant("decide")),
        Err(ServiceError::StaleDirection)
    ));
    let DriveResult::NeedDirection { request: fresh } = advance_one_effect(
        &mut reopened,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap() else {
        panic!("republished fence was not restored");
    };
    assert_eq!(&fresh.expected_revision, reopened.revision());
    assert_ne!(fresh.expected_revision, published.expected_revision);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn resumed_offer_does_not_run_again_after_reopen() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let mut service = open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    let mut offer = InputBatch::no_change();
    offer.wakes.push(WakeEvent::ResourceOffer {
        kind: "slot".into(),
        amount: 2,
    });
    let resumed = advance_one_effect(&mut service, &offer, &CapabilityGrant::default()).unwrap();
    assert!(matches!(resumed, DriveResult::Advanced { .. }));
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    let again = advance_one_effect(&mut reopened, &offer, &CapabilityGrant::default()).unwrap();
    assert!(matches!(again, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct StartThenFail(Arc<AtomicUsize>);

impl EffectPort for StartThenFail {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        let (kind, head) = if n == 0 { ("start", 20) } else { ("fail", 21) };
        Ok(ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(Symbol::new(kind), super::drive_tests::cid(head)),
        })
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

#[test]
fn same_causal_state_keeps_distinct_snapshot_pages() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let phase = open_request.phase.clone();
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(phase.clone()),
        StartThenFail(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        Limits::default(),
    )
    .unwrap();
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    let first = service.snapshots()[0].clone();
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert_eq!(service.snapshots()[0], first);
    let second = service.snapshots()[1].clone();
    let journal = service.inspect().unwrap().journal.snapshots;
    assert_eq!(journal.len(), 2);
    assert_eq!(journal[0].state, journal[1].state);
    assert_ne!(journal[0].covers, journal[1].covers);
    assert_eq!(first.id, journal[0].state);
    assert_ne!(second.id, first.id);
    assert_ne!(second.journal_head, first.journal_head);
    assert_ne!(second.revision, first.revision);
    assert!(matches!(
        service.read_snapshot(&first.id, 0, 8, Some(&first.revision)),
        SnapshotRead::Page { journal_head, .. } if journal_head == first.journal_head
    ));
    assert!(matches!(
        service.read_snapshot(&second.id, 0, 8, Some(&second.revision)),
        SnapshotRead::Page { journal_head, .. } if journal_head == second.journal_head
    ));
    assert!(matches!(
        service.read_snapshot(&first.id, 0, 8, Some(&second.revision)),
        SnapshotRead::Stale { .. }
    ));
    let mut reopened = RoadmapRunnerService::open(
        backend,
        Ready(phase),
        StartThenFail(Arc::clone(&calls)),
        Never,
        open_request,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(reopened.snapshots(), &[first, second]);
    assert_eq!(
        reopened.inspect().unwrap().transition.state,
        PhaseRunState::Failed
    );
    assert!(matches!(
        advance_one_effect(
            &mut reopened,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Ok(DriveResult::Complete { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn receipt_without_success_is_uncertain_on_open() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let execution = open_request.authority.identity.execution.to_string();
    open_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    let journal = ExecutionJournal::new(Arc::clone(&backend), execution, Limits::default());
    let opened = journal.rebuild().unwrap();
    let requested = journal
        .append(
            Some(&opened.head),
            ExecutionRecord::EffectRequested {
                effect_id: "leaf:9".into(),
                kind: "admitted-leaf".into(),
                input: None,
            },
            vec![],
        )
        .unwrap();
    journal
        .append(
            Some(&requested),
            ExecutionRecord::EffectReceipt {
                effect_id: "leaf:9".into(),
                outcome: "start".into(),
                output: None,
            },
            vec![],
        )
        .unwrap();
    let mut reopened = open_on(backend, open_request, Arc::clone(&calls));
    assert!(matches!(
        advance_one_effect(
            &mut reopened,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
