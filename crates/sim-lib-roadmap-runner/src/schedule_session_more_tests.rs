// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use super::{
    Admit, ControlIntent, Liveness, LogicalTime, MAX_ADMITTED, Mode, ModelWorker, ProofOutcome,
    ProofUnit, Session, WakeEvent, WorkerView,
};
use crate::schedule::ResourceAsk;

fn unit(id: &str, mass: u64, resources: &[(&str, u64)]) -> ProofUnit {
    ProofUnit {
        id: id.to_owned(),
        depends_on: Vec::new(),
        capability: "run".to_owned(),
        invalidation_mass: mass,
        learned_cost: 1,
        critical_path_gain: 0,
        novelty_gain: 0,
        resources: resources
            .iter()
            .map(|(kind, amount)| ResourceAsk {
                kind: (*kind).to_owned(),
                amount: *amount,
            })
            .collect(),
    }
}

fn session(units: Vec<ProofUnit>, pools: &[(&str, u64)]) -> Session {
    Session::new(
        units,
        BTreeSet::from(["run".to_owned()]),
        pools
            .iter()
            .map(|(kind, amount)| ((*kind).to_owned(), *amount))
            .collect(),
    )
    .unwrap()
}

#[test]
fn opened_dependency_names_the_new_shortage_not_the_old_waiter() {
    let mut high = unit("high", 100, &[("cpu", 1)]);
    high.depends_on = vec!["gate".to_owned()];
    let mut scheduler = session(
        vec![unit("low", 1, &[("gpu", 1)]), unit("gate", 0, &[]), high],
        &[("cpu", 0), ("gpu", 0)],
    );
    assert_eq!(
        scheduler.admit_next(LogicalTime(1)).unwrap(),
        Admit::Admitted {
            unit: "gate".to_owned()
        }
    );
    scheduler.report("gate", ProofOutcome::Passed).unwrap();
    match scheduler.observe_time(LogicalTime(2)).unwrap() {
        Liveness::Waiting { unit, kind, .. } => {
            assert_eq!(unit, "high");
            assert_eq!(kind, "cpu");
        }
        other => panic!("expected the newly legal shortage, got {other:?}"),
    }
    match scheduler.admit_next(LogicalTime(2)).unwrap() {
        Admit::Waiting { unit, kind, .. } => {
            assert_eq!(unit, "high");
            assert_eq!(kind, "cpu");
        }
        other => panic!("expected admission to try high first, got {other:?}"),
    }
}

#[test]
fn a_filled_waiter_is_progress_until_admission() {
    let mut scheduler = session(vec![unit("a", 1, &[("cpu", 1)])], &[("cpu", 0)]);
    assert!(matches!(
        scheduler.admit_next(LogicalTime(1)).unwrap(),
        Admit::Waiting { .. }
    ));
    scheduler
        .wake(WakeEvent::ResourceOffer {
            kind: "cpu".to_owned(),
            amount: 1,
        })
        .unwrap();
    assert!(matches!(
        scheduler.observe_time(LogicalTime(2)).unwrap(),
        Liveness::Progressing { .. }
    ));
    assert_eq!(
        scheduler.admit_next(LogicalTime(2)).unwrap(),
        Admit::Admitted {
            unit: "a".to_owned()
        }
    );
    assert_eq!(scheduler.pool("cpu"), 0);
    assert_eq!(scheduler.held("a").len(), 1);
}

#[test]
fn slash_ids_keep_separate_leases() {
    let mut scheduler = session(
        vec![unit("a/b", 10, &[("c", 1)]), unit("a", 1, &[("b/c", 1)])],
        &[("c", 1), ("b/c", 1)],
    );
    assert_eq!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Admitted {
            unit: "a/b".to_owned()
        }
    );
    assert_eq!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Admitted {
            unit: "a".to_owned()
        }
    );
    assert_eq!(scheduler.pool("c"), 0);
    assert_eq!(scheduler.pool("b/c"), 0);
    scheduler.report("a/b", ProofOutcome::Passed).unwrap();
    assert_eq!(scheduler.pool("c"), 1);
    assert_eq!(scheduler.pool("b/c"), 0);
    assert!(scheduler.held("a/b").is_empty());
    assert_eq!(scheduler.held("a").len(), 1);
    assert_eq!(scheduler.held("a")[0].kind, "b/c");
    assert_eq!(scheduler.mode(), Mode::Running);
}

struct HangWorker {
    entered: Arc<Barrier>,
}

impl ModelWorker for HangWorker {
    fn outcome(&self, _: &str) -> WorkerView {
        self.entered.wait();
        loop {
            thread::park();
        }
    }
}

#[test]
fn admission_stops_at_the_bound_and_stop_still_returns() {
    let units = (0..MAX_ADMITTED + 1)
        .map(|index| unit(&format!("u{index}"), 1, &[]))
        .collect();
    let mut scheduler = session(units, &[]);
    for _ in 0..MAX_ADMITTED {
        assert!(matches!(
            scheduler.admit_next(LogicalTime(0)).unwrap(),
            Admit::Admitted { .. }
        ));
    }
    assert_eq!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Refused("admission-bound")
    );
    assert_eq!(scheduler.work_len(), MAX_ADMITTED);
    let ack = scheduler
        .submit(ControlIntent::Stop, LogicalTime(1))
        .unwrap();
    assert_eq!(ack.mode, Mode::Stopped);
    assert_eq!(scheduler.work_len(), 0);
}

#[test]
fn stop_acks_while_the_worker_has_not_returned() {
    let entered = Arc::new(Barrier::new(2));
    let worker = HangWorker {
        entered: Arc::clone(&entered),
    };
    let mut scheduler = session(vec![unit("stuck", 1, &[])], &[]);
    assert!(matches!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Admitted { .. }
    ));
    thread::spawn(move || {
        let _view = worker.outcome("stuck");
    });
    entered.wait();
    let started = Instant::now();
    let ack = scheduler
        .submit(ControlIntent::Stop, LogicalTime(1))
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(ack.mode, Mode::Stopped);
    assert!(elapsed.as_millis() < 1_000, "stop took {elapsed:?}");
    assert!(!scheduler.os_terminated());
    assert_eq!(scheduler.work_len(), 0);
    assert!(scheduler.failure_set(10).unwrap().failures.is_empty());
}
