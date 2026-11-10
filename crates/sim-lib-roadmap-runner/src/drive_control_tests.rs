// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Stop is acknowledged while an effect call has not returned.

use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Instant;

use sim_lib_journal::MemoryBackend;
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{AttemptId, ExecutionEvent, Observation};

use super::drive_tests::{Never, Ready, cid, request};
use crate::{
    CapabilityGrant, ControlIntent, DriveResult, EffectPort, ExecutionIdentity, ExecutionRecord,
    InputBatch, Limits, OpenRequest, Prepared, RoadmapRunnerService, ServiceError, WakeEvent,
};

struct HangEffects {
    entered: Arc<Barrier>,
}

impl EffectPort for HangEffects {
    fn invoke(
        &self,
        _: &ExecutionIdentity,
        _: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&sim_kernel::Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.entered.wait();
        loop {
            thread::park();
        }
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

fn open_hang(
    request: OpenRequest,
    entered: Arc<Barrier>,
) -> RoadmapRunnerService<MemoryBackend, Ready, HangEffects, Never> {
    let phase = request.phase.clone();
    RoadmapRunnerService::open(
        Arc::new(MemoryBackend::new()),
        Ready(phase),
        HangEffects { entered },
        Never,
        request,
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn stop_acks_while_an_effect_has_not_returned() {
    let entered = Arc::new(Barrier::new(2));
    let service = Arc::new(Mutex::new(open_hang(request(), Arc::clone(&entered))));
    let running = Arc::clone(&service);
    thread::spawn(move || {
        let _stuck = crate::conduct_one(
            &running,
            &InputBatch::conduct(),
            &CapabilityGrant::default(),
        );
    });
    entered.wait();
    let mut stop = InputBatch::no_change();
    stop.wakes.push(WakeEvent::Control(ControlIntent::Stop));
    let started = Instant::now();
    let stepped = service
        .lock()
        .expect("service")
        .prepare_effect(&stop, &CapabilityGrant::default())
        .unwrap();
    let elapsed = started.elapsed();
    let Prepared::Done(result) = stepped else {
        panic!("stop should finish without invoking, got {stepped:?}");
    };
    assert!(
        matches!(*result, DriveResult::Advanced { .. }),
        "{result:?}"
    );
    assert!(elapsed.as_millis() < 1_000, "stop took {elapsed:?}");
}

struct ReleaseEffects {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl EffectPort for ReleaseEffects {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        _: Option<&sim_kernel::Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        self.entered.wait();
        self.release.wait();
        Ok(ExecutionEvent {
            execution: identity.execution.clone(),
            phase: phase.clone(),
            attempt: AttemptId::new("attempt").unwrap(),
            observation: Observation::new(sim_kernel::Symbol::new("start"), cid(20)),
        })
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(vec![])
    }
}

#[test]
fn a_completed_effect_commits_after_stop() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let service = Arc::new(Mutex::new(
        RoadmapRunnerService::open(
            Arc::new(MemoryBackend::new()),
            Ready(request().phase.clone()),
            ReleaseEffects {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            },
            Never,
            request(),
            Limits::default(),
        )
        .unwrap(),
    ));
    let running = Arc::clone(&service);
    let worker = thread::spawn(move || {
        crate::conduct_one(
            &running,
            &InputBatch::conduct(),
            &CapabilityGrant::default(),
        )
    });
    entered.wait();
    let mut stop = InputBatch::no_change();
    stop.wakes.push(WakeEvent::Control(ControlIntent::Stop));
    let stepped = service
        .lock()
        .expect("service")
        .prepare_effect(&stop, &CapabilityGrant::default())
        .unwrap();
    assert!(
        matches!(stepped, Prepared::Done(ref result) if matches!(**result, DriveResult::Advanced { .. })),
        "{stepped:?}"
    );
    release.wait();
    let committed = worker.join().expect("worker").expect("commit");
    assert!(
        matches!(
            committed,
            DriveResult::Advanced { .. } | DriveResult::Complete { .. }
        ),
        "{committed:?}"
    );
    let mut guard = service.lock().expect("service");
    let records = guard.inspect().expect("inspect").journal.records;
    let stopped = records.iter().position(|record| {
        matches!(record, ExecutionRecord::SemanticDelta { cause, .. } if cause == "control-stop")
    });
    let receipt = records
        .iter()
        .position(|record| matches!(record, ExecutionRecord::EffectReceipt { .. }));
    assert!(
        stopped.is_some() && receipt.is_some() && stopped < receipt,
        "stop {stopped:?} receipt {receipt:?}"
    );
    assert!(
        !records.iter().any(|record| {
            matches!(record, ExecutionRecord::SemanticDelta { cause, .. } if cause == "effect-uncertain")
        })
    );
    let again = guard
        .prepare_effect(&InputBatch::conduct(), &CapabilityGrant::default())
        .expect("stopped session");
    assert!(matches!(again, Prepared::Done(_)), "{again:?}");
}
