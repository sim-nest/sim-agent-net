// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one advance performs one effect or one pure transition.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::{
    Admission, CommitAction, GuardedAdmission, JournalBackend, JournalError, JournalHead,
    JournalObject, Lease, MemoryBackend, StoredDatumRef, StoredState,
};
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{AttemptId, ExecutionEvent, Observation, PhaseRunState};
use sim_work_core::ResourceNeed;

use super::drive_tests::{CountEffects, Never, Ready, cid, question, reply, request};
use crate::{
    CapabilityGrant, DirectionKind, DriveResult, EffectPort, ExecutionIdentity, ExecutionJournal,
    ExecutionJournalError, ExecutionRecord, InputBatch, ReadinessPort, RoadmapRunnerService,
    SemanticChange, ServiceError, WakeEvent,
};

struct StoredReject {
    calls: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<ExecutionEvent>>>,
}

impl EffectPort for StoredReject {
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
            observation: Observation::new(Symbol::new("nope"), cid(21)),
        };
        self.events.lock().unwrap().push(event.clone());
        Ok(event)
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

struct StartThenFail(Arc<AtomicUsize>);

impl EffectPort for StartThenFail {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
    ) -> Result<ExecutionEvent, ServiceError> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        let (kind, head) = if n == 0 { ("start", 20) } else { ("fail", 21) };
        Ok(ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(Symbol::new(kind), cid(head)),
        })
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

/// Refuses one snapshot admission. Earlier records stay on the inner backend.
struct FailSnapshot {
    inner: Arc<MemoryBackend>,
    snapshots: AtomicUsize,
    /// Zero-based count of snapshot admissions to refuse.
    fail_at: usize,
}

impl JournalBackend for FailSnapshot {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        self.inner.acquire_lease()
    }

    fn read_state(&self) -> Result<StoredState, JournalError> {
        self.inner.read_state()
    }

    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
        let snapshot = admission
            .entries()
            .iter()
            .any(|entry| entry.kind == Symbol::qualified("roadmap-execution", "snapshot"));
        if snapshot && self.snapshots.fetch_add(1, Ordering::SeqCst) == self.fail_at {
            return Err(JournalError::WriteRefused("snapshot"));
        }
        self.inner.admit(admission)
    }

    fn admit_then(
        &self,
        admission: Admission,
        action: &mut dyn CommitAction,
    ) -> Result<GuardedAdmission, JournalError> {
        self.inner.admit_then(admission, action)
    }

    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
        self.inner.put_datum(object)
    }

    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
        self.inner.get_datum(meaning)
    }

    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
        self.inner.rebuild_datum_index()
    }
}

fn uncertain_marker<B, R, E, C>(service: &RoadmapRunnerService<B, R, E, C>) -> bool
where
    B: JournalBackend,
    R: ReadinessPort,
    E: EffectPort,
    C: crate::CancellationPort,
{
    service.inspect().unwrap().journal.records.iter().any(|record| {
        matches!(record, ExecutionRecord::SemanticDelta { cause, .. } if cause == "effect-uncertain")
    })
}

#[test]
fn rejected_receipt_reopens_as_uncertain() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let open_request = request();
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(open_request.phase.clone()),
        StoredReject {
            calls: Arc::clone(&calls),
            events: Arc::clone(&events),
        },
        Never,
        open_request.clone(),
        crate::Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        service.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Err(ServiceError::Reducer(_))
    ));
    assert!(uncertain_marker(&service));
    let reopened = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        StoredReject {
            calls: Arc::clone(&calls),
            events,
        },
        Never,
        open_request,
        crate::Limits::default(),
    );
    assert!(reopened.is_ok());
    let mut reopened = reopened.unwrap();
    assert!(matches!(
        reopened.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn informational_empty_answer_refuses_a_different_reply() {
    let mut open_request = request();
    open_request.direction = Some(question(None));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request,
        crate::Limits::default(),
    )
    .unwrap();
    let DriveResult::NeedDirection { request } = service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap()
    else {
        panic!("direction was not published");
    };
    let len = service.inspect().unwrap().journal.records.len();
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&request, DirectionKind::Decision, ""));
    let noted = service
        .advance_one_effect(&batch, &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(noted, DriveResult::Quiescent { .. }));
    assert_eq!(service.inspect().unwrap().journal.records.len(), len);
    batch.reply = Some(reply(&request, DirectionKind::Decision, "later"));
    assert!(matches!(
        service.advance_one_effect(&batch, &CapabilityGrant::default()),
        Err(ServiceError::ReplayedDirection)
    ));
    batch.reply = Some(reply(&request, DirectionKind::Decision, ""));
    let again = service
        .advance_one_effect(&batch, &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(again, DriveResult::Quiescent { .. }));
    assert_eq!(service.inspect().unwrap().journal.records.len(), len);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Planned
    );
}

#[test]
fn accepted_direction_records_one_cause() {
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::new(AtomicUsize::new(0))),
        Never,
        open_request,
        crate::Limits::default(),
    )
    .unwrap();
    let DriveResult::NeedDirection { request } = service
        .advance_one_effect(&InputBatch::conduct(), &super::drive_tests::grant("decide"))
        .unwrap()
    else {
        panic!("direction was not published");
    };
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&request, DirectionKind::Decision, "yes"));
    service
        .advance_one_effect(&batch, &super::drive_tests::grant("decide"))
        .unwrap();
    let records = service.inspect().unwrap().journal.records;
    let mut paired = 0;
    for record in &records {
        if let ExecutionRecord::SemanticDelta { cause, changes, .. } = record {
            assert_ne!(cause, "direction-answer");
            if cause == "direction-decision" {
                paired += 1;
                assert!(
                    changes
                        .iter()
                        .any(|change| change.key == "direction-decision")
                );
                assert!(
                    changes
                        .iter()
                        .any(|change| change.key == "direction-answer")
                );
            }
        }
    }
    assert_eq!(paired, 1);
}

#[test]
fn torn_direction_refuses_the_original_answer() {
    let backend = Arc::new(MemoryBackend::new());
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let execution = open_request.authority.identity.execution.to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let published = {
        let mut service = RoadmapRunnerService::open(
            Arc::clone(&backend),
            Ready(open_request.phase.clone()),
            CountEffects(Arc::clone(&calls)),
            Never,
            open_request.clone(),
            crate::Limits::default(),
        )
        .unwrap();
        let DriveResult::NeedDirection { request } = service
            .advance_one_effect(&InputBatch::conduct(), &super::drive_tests::grant("decide"))
            .unwrap()
        else {
            panic!("direction was not published");
        };
        let journal =
            ExecutionJournal::new(Arc::clone(&backend), execution, crate::Limits::default());
        let rebuilt = journal.rebuild().unwrap();
        journal
            .append_delta(
                &rebuilt.head,
                "direction-decision",
                vec![SemanticChange {
                    key: "direction-decision".into(),
                    before: None,
                    after: Some(cid(40)),
                }],
                BTreeSet::new(),
            )
            .unwrap();
        request
    };
    let kept = ExecutionJournal::new(
        Arc::clone(&backend),
        open_request.authority.identity.execution.to_string(),
        crate::Limits::default(),
    )
    .rebuild()
    .unwrap()
    .records
    .len();
    let mut reopened = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        CountEffects(calls),
        Never,
        open_request,
        crate::Limits::default(),
    )
    .unwrap();
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&published, DirectionKind::Decision, "yes"));
    assert!(matches!(
        reopened.advance_one_effect(&batch, &super::drive_tests::grant("decide")),
        Err(ServiceError::ReplayedDirection)
    ));
    assert_eq!(reopened.inspect().unwrap().journal.records.len(), kept);
}

#[test]
fn snapshot_miss_after_commit_returns_complete() {
    let backend = Arc::new(FailSnapshot {
        inner: Arc::new(MemoryBackend::new()),
        snapshots: AtomicUsize::new(0),
        fail_at: 1,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        StartThenFail(Arc::clone(&calls)),
        Never,
        open_request,
        crate::Limits::default(),
    )
    .unwrap();
    let started = service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(started, DriveResult::Advanced { .. }));
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    assert!(matches!(
        service.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Err(ServiceError::Journal(ExecutionJournalError::Journal(
            JournalError::WriteRefused("snapshot")
        )))
    ));
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Failed
    );
    assert!(!uncertain_marker(&service));
    let finished = service
        .advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(finished, DriveResult::Complete { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn snapshot_miss_on_resume_does_not_invoke_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    let backend = Arc::new(FailSnapshot {
        inner: Arc::new(MemoryBackend::new()),
        snapshots: AtomicUsize::new(0),
        fail_at: 0,
    });
    let mut service = RoadmapRunnerService::open(
        Arc::clone(&backend),
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request.clone(),
        crate::Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        service.advance_one_effect(&InputBatch::conduct(), &CapabilityGrant::default()),
        Ok(DriveResult::Blocked { .. })
    ));
    let mut offer = InputBatch::no_change();
    offer.wakes.push(WakeEvent::ResourceOffer {
        kind: "slot".into(),
        amount: 2,
    });
    assert!(matches!(
        service.advance_one_effect(&offer, &CapabilityGrant::default()),
        Err(ServiceError::Journal(ExecutionJournalError::Journal(
            JournalError::WriteRefused("snapshot")
        )))
    ));
    assert_eq!(
        service.inspect().unwrap().transition.state,
        PhaseRunState::Running
    );
    assert!(!uncertain_marker(&service));
    let again = service
        .advance_one_effect(&offer, &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(again, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut reopened = RoadmapRunnerService::open(
        backend,
        Ready(open_request.phase.clone()),
        CountEffects(Arc::clone(&calls)),
        Never,
        open_request,
        crate::Limits::default(),
    )
    .unwrap();
    let restored = reopened
        .advance_one_effect(&offer, &CapabilityGrant::default())
        .unwrap();
    assert!(matches!(restored, DriveResult::Quiescent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
