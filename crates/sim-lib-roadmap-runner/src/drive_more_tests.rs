// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: one advance performs one effect or one pure transition.

use super::drive_harness::advance_one_effect;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sim_kernel::{Cx, Datum, DefaultFactory, HandleSeed, NoopEvalPolicy, Shape, Symbol};
use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{AttemptId, ExecutionEvent, Observation};
use sim_work_core::ResourceNeed;

use super::drive_tests::{
    Ready, cid, grant, open_count, open_fail, open_fail_on, open_reject, question, reply, request,
};
use crate::{
    CapabilityGrant, ControlIntent, DirectionHandoff, DirectionKind, DriveResult, EffectPort,
    ExecutionIdentity, ExecutionRecord, ExternalLineageRef, InputBatch, Limits, LineageView,
    RoadmapRunnerService, ServiceError, SnapshotRead, WakeEvent, direction_shape,
};

#[test]
fn missing_begin_refuses_and_lineage_names_gaps() {
    let mut open_request = request();
    open_request.handoff = Some(DirectionHandoff {
        exact_source: cid(11),
        candidate: cid(12),
        selection: cid(13),
        begin: None,
        external_lineage: vec![
            ExternalLineageRef {
                relation: Symbol::new("source"),
                owner: Symbol::new("owner-a"),
                shape: Symbol::new("text"),
                value: cid(14),
            },
            ExternalLineageRef {
                relation: Symbol::new("commission"),
                owner: Symbol::new("owner-b"),
                shape: Symbol::new("text"),
                value: cid(15),
            },
        ],
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    assert_eq!(service.handoff().unwrap().selection, cid(13));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::MissingBegin)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let view = service.lineage(&BTreeSet::from([cid(14)]));
    assert!(matches!(&view[0], LineageView::Present(item) if item.value == cid(14)));
    assert!(matches!(&view[1], LineageView::Gap { value } if value == &cid(15)));
}

#[test]
fn uncertain_effect_is_not_invoked_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_fail(request(), Arc::clone(&calls));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::Adapter(_))
    ));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        service
            .inspect()
            .unwrap()
            .journal
            .records
            .iter()
            .any(|record| { matches!(record, ExecutionRecord::EffectRequested { .. }) })
    );
    assert!(
        !service
            .inspect()
            .unwrap()
            .journal
            .records
            .iter()
            .any(|record| matches!(record, ExecutionRecord::EffectReceipt { .. }))
    );
}

#[test]
fn pause_blocks_conduct_until_resume() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(request(), Arc::clone(&calls));
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut resume = InputBatch::no_change();
    resume.wakes.push(WakeEvent::Control(ControlIntent::Resume));
    advance_one_effect(&mut service, &resume, &CapabilityGrant::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn one_call_records_one_opaque_product_step() {
    for name in ["step-a", "step-b", "step-c", "step-d"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut service = open_count(request(), Arc::clone(&calls));
        let mut batch = InputBatch::conduct();
        batch.product = Some(Symbol::new(name));
        advance_one_effect(&mut service, &batch, &CapabilityGrant::default()).unwrap();
        let kinds: Vec<_> = service
            .inspect()
            .unwrap()
            .journal
            .records
            .iter()
            .filter_map(|record| match record {
                ExecutionRecord::EffectRequested { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![Symbol::new(name).to_string()]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn direction_shapes_are_distinct() {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x25),
    );
    let advice = direction_shape(DirectionKind::Advice);
    let decision = direction_shape(DirectionKind::Decision);
    assert_eq!(
        Shape::is_subshape_of(&advice, &mut cx, &decision).unwrap(),
        Some(false)
    );
    assert_eq!(
        Shape::is_subshape_of(&decision, &mut cx, &decision).unwrap(),
        Some(true)
    );
}

#[test]
fn missing_begin_blocks_an_offer_and_conduct() {
    let mut open_request = request();
    open_request.resources.push(ResourceNeed {
        kind: Symbol::new("slot"),
        requirement: Datum::Nil,
    });
    open_request.handoff = Some(DirectionHandoff {
        exact_source: cid(11),
        candidate: cid(12),
        selection: cid(13),
        begin: None,
        external_lineage: vec![],
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
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
        kind: "slot".into(),
        amount: 2,
    });
    assert!(matches!(
        advance_one_effect(&mut service, &offer, &CapabilityGrant::default()),
        Err(ServiceError::MissingBegin)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(service.leases().is_empty());
    assert_eq!(service.handoff().unwrap().selection, cid(13));
}

#[test]
fn unresolved_effect_stays_uncertain_after_reopen() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = open_fail_on(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::Adapter(_))
    ));
    let mut again = open_fail_on(backend, open_request, Arc::clone(&calls));
    assert!(matches!(
        advance_one_effect(
            &mut again,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        again
            .inspect()
            .unwrap()
            .journal
            .records
            .iter()
            .any(|record| matches!(record, ExecutionRecord::EffectRequested { .. }))
    );
}

#[test]
fn reducer_rejection_does_not_invoke_again() {
    let backend = Arc::new(MemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let mut service = open_reject(
        Arc::clone(&backend),
        open_request.clone(),
        Arc::clone(&calls),
    );
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::Reducer(_))
    ));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    let mut again = open_reject(backend, open_request, Arc::clone(&calls));
    assert!(matches!(
        advance_one_effect(
            &mut again,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let records = again.inspect().unwrap().journal.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record, ExecutionRecord::EffectRequested { .. }))
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .any(|record| matches!(record, ExecutionRecord::EffectReceipt { .. }))
    );
}

struct CancelEffects(Arc<AtomicUsize>);
impl EffectPort for CancelEffects {
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
            observation: Observation::new(Symbol::new("cancel"), cid(22)),
        })
    }
    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

#[test]
fn cancelled_observation_does_not_invoke_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let open_request = request();
    let phase = open_request.phase.clone();
    let mut service = RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(phase),
        CancelEffects(Arc::clone(&calls)),
        super::drive_tests::Never,
        open_request,
        Limits::default(),
    )
    .unwrap();
    let first = advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    );
    assert!(first.is_err());
    assert!(!matches!(first, Err(ServiceError::EffectUncertain)));
    assert!(matches!(
        advance_one_effect(
            &mut service,
            &InputBatch::conduct(),
            &CapabilityGrant::default()
        ),
        Err(ServiceError::EffectUncertain)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let records = service.inspect().unwrap().journal.records;
    assert!(
        records
            .iter()
            .any(|record| matches!(record, ExecutionRecord::EffectReceipt { .. }))
    );
    assert!(!records.iter().any(|record| {
        matches!(record, ExecutionRecord::StateTransition { to, .. } if to == "cancelled")
    }));
}

#[test]
fn advice_question_cannot_authorize() {
    let mut open_request = request();
    let mut asked = question(Some(Symbol::new("decide")));
    asked.answer_kind = DirectionKind::Advice;
    open_request.direction = Some(asked);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let DriveResult::NeedDirection { request } =
        advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap()
    else {
        panic!("direction was not published");
    };
    let len = service.inspect().unwrap().journal.records.len();
    let mut batch = InputBatch::no_change();
    batch.reply = Some(reply(&request, DirectionKind::Advice, "maybe"));
    assert!(matches!(
        advance_one_effect(&mut service, &batch, &grant("decide")),
        Err(ServiceError::AdvisoryDirection)
    ));
    assert_eq!(service.inspect().unwrap().journal.records.len(), len);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn control_republishes_the_direction_fence() {
    let mut open_request = request();
    open_request.direction = Some(question(Some(Symbol::new("decide"))));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(open_request, Arc::clone(&calls));
    let DriveResult::NeedDirection { request } =
        advance_one_effect(&mut service, &InputBatch::conduct(), &grant("decide")).unwrap()
    else {
        panic!("direction was not published");
    };
    let mut pause = InputBatch::no_change();
    pause.wakes.push(WakeEvent::Control(ControlIntent::Pause));
    advance_one_effect(&mut service, &pause, &CapabilityGrant::default()).unwrap();
    let mut stale = InputBatch::no_change();
    stale.reply = Some(reply(&request, DirectionKind::Decision, "yes"));
    assert!(matches!(
        advance_one_effect(&mut service, &stale, &grant("decide")),
        Err(ServiceError::StaleDirection)
    ));
    let DriveResult::NeedDirection { request: fresh } = advance_one_effect(
        &mut service,
        &InputBatch::no_change(),
        &CapabilityGrant::default(),
    )
    .unwrap() else {
        panic!("republished fence was not handed out");
    };
    assert_eq!(&fresh.expected_revision, service.revision());
    assert_ne!(fresh.expected_revision, request.expected_revision);
    let mut accepted = InputBatch::no_change();
    accepted.reply = Some(reply(&fresh, DirectionKind::Decision, "yes"));
    assert!(matches!(
        advance_one_effect(&mut service, &accepted, &grant("decide")),
        Ok(DriveResult::Advanced { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn effect_snapshot_is_the_journal_state() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = open_count(request(), Arc::clone(&calls));
    advance_one_effect(
        &mut service,
        &InputBatch::conduct(),
        &CapabilityGrant::default(),
    )
    .unwrap();
    let verified = service
        .inspect()
        .unwrap()
        .journal
        .snapshots
        .last()
        .unwrap()
        .clone();
    let page = service.snapshots().last().unwrap().clone();
    assert_eq!(page.id, verified.state);
    assert_eq!(page.journal_head, verified.covers);
    assert_eq!(page.rows.last().unwrap().key, "evidence-set");
    assert_eq!(page.rows.last().unwrap().value, verified.evidence_set);
    assert!(matches!(
        service.read_snapshot(&verified.state, 0, 4, None),
        SnapshotRead::Page { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
