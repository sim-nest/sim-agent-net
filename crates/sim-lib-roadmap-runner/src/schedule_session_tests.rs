// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use super::{
    Admit, ControlIntent, Liveness, LogicalTime, MAX_CLEANUP_BEATS, Mode, ModelWorker,
    ProofOutcome, ProofUnit, ScheduleError, Session, WakeCause, WakeEvent, WorkerView,
};
use crate::schedule::ResourceAsk;

struct CountingWorker {
    calls: Arc<AtomicUsize>,
    view: WorkerView,
}

impl ModelWorker for CountingWorker {
    fn outcome(&self, _id: &str) -> WorkerView {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.view
    }
}

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

fn p95(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    let rank = (samples.len() * 95).div_ceil(100);
    samples[rank - 1]
}

#[test]
fn a_wait_releases_only_that_units_leases_and_frees_the_slot() {
    let mut scheduler = session(
        vec![
            unit("a", 10, &[("cpu", 1), ("gpu", 1)]),
            unit("b", 1, &[("disk", 1)]),
        ],
        &[("cpu", 1), ("gpu", 0), ("disk", 1)],
    );
    let admitted = scheduler.admit_next(LogicalTime(1)).unwrap();
    assert_eq!(
        admitted,
        Admit::Admitted {
            unit: "b".to_owned()
        }
    );
    assert!(scheduler.held("a").is_empty());
    assert_eq!(scheduler.pool("cpu"), 1);
    assert_eq!(scheduler.held("b").len(), 1);
    assert_eq!(scheduler.held("b")[0].kind, "disk");
    assert_eq!(scheduler.pool("disk"), 0);
    match scheduler.observe_time(LogicalTime(1)).unwrap() {
        Liveness::Waiting { unit, kind, .. } => {
            assert_eq!(unit, "a");
            assert_eq!(kind, "gpu");
        }
        other => panic!("expected a wait, got {other:?}"),
    }
}

#[test]
fn a_later_shortage_keeps_the_first_wait_and_names_the_short_resource() {
    let mut scheduler = session(
        vec![unit("a", 1, &[("cpu", 1), ("gpu", 1)])],
        &[("cpu", 0), ("gpu", 0)],
    );
    match scheduler.admit_next(LogicalTime(1)).unwrap() {
        Admit::Waiting { unit, kind, since } => {
            assert_eq!(unit, "a");
            assert_eq!(kind, "cpu");
            assert_eq!(since, LogicalTime(1));
        }
        other => panic!("expected the first shortage, got {other:?}"),
    }
    scheduler
        .wake(WakeEvent::ResourceOffer {
            kind: "cpu".to_owned(),
            amount: 1,
        })
        .unwrap();
    match scheduler.admit_next(LogicalTime(2)).unwrap() {
        Admit::Waiting { unit, kind, since } => {
            assert_eq!(unit, "a");
            assert_eq!(kind, "gpu");
            assert_eq!(since, LogicalTime(1));
        }
        other => panic!("expected the moved shortage, got {other:?}"),
    }
    match scheduler.observe_time(LogicalTime(2)).unwrap() {
        Liveness::Waiting { unit, kind, since } => {
            assert_eq!(unit, "a");
            assert_eq!(kind, "gpu");
            assert_eq!(since, LogicalTime(1));
        }
        other => panic!("expected the recorded wait, got {other:?}"),
    }
    assert_eq!(scheduler.pool("cpu"), 1);
    assert!(scheduler.held("a").is_empty());
    assert_eq!(scheduler.work_len(), 0);
}

#[test]
fn repeated_stop_while_cleanup_is_open_does_not_append() {
    let mut scheduler = session(vec![unit("a", 1, &[("disk", 1)])], &[("disk", 1)]);
    assert!(matches!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Admitted { .. }
    ));
    let ack = scheduler
        .submit(ControlIntent::Stop, LogicalTime(1))
        .unwrap();
    assert_eq!(ack.mode, Mode::Stopping);
    assert_eq!(scheduler.held("a").len(), 1);
    let again = scheduler
        .submit(ControlIntent::Stop, LogicalTime(2))
        .unwrap();
    assert_eq!(again.mode, Mode::Stopping);
    assert_eq!(scheduler.control_log().len(), 1);
    assert_eq!(scheduler.control_log()[0].at, LogicalTime(1));
    assert!(!scheduler.os_terminated());
}

#[test]
fn stop_ack_does_not_call_a_blocked_worker() {
    let mut samples = Vec::with_capacity(100);
    for index in 0..100 {
        let mut scheduler = session(vec![unit(&format!("w{index}"), 1, &[])], &[]);
        assert!(matches!(
            scheduler.admit_next(LogicalTime(0)).unwrap(),
            Admit::Admitted { .. }
        ));
        let started = Instant::now();
        let ack = scheduler
            .submit(ControlIntent::Stop, LogicalTime(1))
            .unwrap();
        samples.push(started.elapsed().as_micros());
        assert_eq!(ack.mode, Mode::Stopped);
        assert!(!scheduler.os_terminated());
        assert!(scheduler.failure_set(10).unwrap().failures.is_empty());
        assert_eq!(scheduler.control_log().len(), 1);
        assert_eq!(scheduler.work_len(), 0);
    }
    let measured = p95(&mut samples);
    eprintln!("stop-ack-p95-us={measured}");
    assert!(measured < 1_000_000, "stop p95 {measured}us");
}

#[test]
fn blocked_queue_does_not_block_control() {
    let units = (0..8)
        .map(|index| unit(&format!("q{index}"), 1, &[]))
        .collect();
    let mut scheduler = session(units, &[]);
    for _ in 0..8 {
        assert!(matches!(
            scheduler.admit_next(LogicalTime(0)).unwrap(),
            Admit::Admitted { .. }
        ));
    }
    assert_eq!(scheduler.work_len(), 8);
    assert!(scheduler.control_log().is_empty());
    let ack = scheduler
        .submit(ControlIntent::Stop, LogicalTime(2))
        .unwrap();
    assert_eq!(ack.mode, Mode::Stopped);
    assert_eq!(scheduler.work_len(), 0);
    assert_eq!(scheduler.control_log()[0].intent, ControlIntent::Stop);
}

#[test]
fn drain_admits_no_new_work_and_lets_the_admitted_unit_finish() {
    let mut scheduler = session(vec![unit("a", 2, &[]), unit("b", 1, &[])], &[]);
    assert_eq!(
        scheduler.admit_next(LogicalTime(0)).unwrap(),
        Admit::Admitted {
            unit: "a".to_owned()
        }
    );
    let ack = scheduler
        .submit(ControlIntent::Drain, LogicalTime(1))
        .unwrap();
    assert_eq!(ack.mode, Mode::Draining);
    assert_eq!(
        scheduler.admit_next(LogicalTime(1)).unwrap(),
        Admit::Refused("draining")
    );
    scheduler.report("a", ProofOutcome::Passed).unwrap();
    assert_eq!(scheduler.work_len(), 0);
    assert!(scheduler.failure_set(10).unwrap().failures.is_empty());
}

#[test]
fn pause_performs_no_work_and_resume_reobserves_expired_facts() {
    let mut scheduler = session(vec![unit("a", 1, &[])], &[]);
    scheduler.admit_next(LogicalTime(0)).unwrap();
    scheduler.insert_fact("fact", LogicalTime(0), 10).unwrap();
    scheduler.report("a", ProofOutcome::Failed).unwrap();
    scheduler
        .submit(ControlIntent::Pause, LogicalTime(1))
        .unwrap();
    assert_eq!(
        scheduler.admit_next(LogicalTime(2)).unwrap(),
        Admit::Refused("paused")
    );
    assert!(scheduler.report("a", ProofOutcome::Passed).is_err());
    let ack = scheduler
        .submit(ControlIntent::Resume, LogicalTime(100))
        .unwrap();
    assert_eq!(ack.expired, BTreeSet::from(["fact".to_owned()]));
    assert!(ack.retained.is_empty());
    let frontier = scheduler.failure_set(10).unwrap();
    assert_eq!(frontier.failures.len(), 1);
    match scheduler.observe_time(LogicalTime(100)).unwrap() {
        Liveness::Quiescent { .. } | Liveness::Progressing { .. } => {}
        Liveness::Stalled { .. } => panic!("pause was classified as a stall"),
        other => panic!("unexpected liveness {other:?}"),
    }
}

#[test]
fn stall_is_not_retry_or_failure() {
    let mut samples = Vec::with_capacity(100);
    for index in 0..100 {
        let mut scheduler = session(vec![unit(&format!("s{index}"), 1, &[])], &[]);
        scheduler.admit_next(LogicalTime(0)).unwrap();
        if index == 0 {
            assert!(matches!(
                scheduler.observe_time(LogicalTime(29)).unwrap(),
                Liveness::Progressing { .. }
            ));
        }
        let started = Instant::now();
        let live = scheduler.observe_time(LogicalTime(30)).unwrap();
        samples.push(started.elapsed().as_micros());
        assert!(matches!(live, Liveness::Stalled { .. }), "{live:?}");
        assert!(scheduler.failure_set(10).unwrap().failures.is_empty());
        assert_eq!(scheduler.work_len(), 1);
        assert_eq!(scheduler.admit_next(LogicalTime(30)).unwrap(), Admit::Idle);
    }
    let measured = p95(&mut samples);
    eprintln!("stall-class-p95-us={measured}");
    assert!(measured < 30_000_000, "stall p95 {measured}us");
}

#[test]
fn cleanup_beats_have_no_five_tick_silence_and_do_not_claim_process_death() {
    let mut scheduler = session(vec![unit("a", 1, &[("disk", 1)])], &[("disk", 1)]);
    scheduler.admit_next(LogicalTime(0)).unwrap();
    let ack = scheduler
        .submit(ControlIntent::Stop, LogicalTime(0))
        .unwrap();
    assert_eq!(ack.mode, Mode::Stopping);
    assert_eq!(scheduler.held("a").len(), 1);
    let live = scheduler.observe_time(LogicalTime(23)).unwrap();
    let Liveness::Cleaning { beats } = live else {
        panic!("expected cleanup, got {live:?}");
    };
    let ticks: Vec<u64> = beats.iter().map(|beat| beat.0).collect();
    assert_eq!(ticks.first().copied(), Some(0));
    for pair in ticks.windows(2) {
        assert!(pair[1] - pair[0] < 5, "{ticks:?}");
    }
    assert!(ticks.last().copied().unwrap() - ticks[ticks.len() - 2] < 5);
    scheduler.close_modeled_resources().unwrap();
    assert_eq!(scheduler.mode(), Mode::Stopped);
    assert!(scheduler.held("a").is_empty());
    assert!(!scheduler.os_terminated());
    assert!(matches!(
        scheduler.observe_time(LogicalTime(23)).unwrap(),
        Liveness::Stopped {
            modeled_resources_closed: true
        }
    ));
}

#[test]
fn cleanup_starts_at_the_stop_tick_and_a_long_span_continues() {
    let mut late = session(vec![unit("a", 1, &[("disk", 1)])], &[("disk", 1)]);
    late.admit_next(LogicalTime(0)).unwrap();
    late.submit(ControlIntent::Stop, LogicalTime(20_000))
        .unwrap();
    match late.observe_time(LogicalTime(20_000)).unwrap() {
        Liveness::Cleaning { beats } => assert_eq!(beats, vec![LogicalTime(20_000)]),
        other => panic!("expected the stop tick, got {other:?}"),
    }
    assert_eq!(late.mode(), Mode::Stopping);
    assert!(!late.os_terminated());

    let mut scheduler = session(vec![unit("a", 1, &[("disk", 1)])], &[("disk", 1)]);
    scheduler.admit_next(LogicalTime(0)).unwrap();
    scheduler
        .submit(ControlIntent::Stop, LogicalTime(0))
        .unwrap();
    let first = match scheduler.observe_time(LogicalTime(20_000)).unwrap() {
        Liveness::Cleaning { beats } => beats,
        other => panic!("expected the first cleanup span, got {other:?}"),
    };
    assert_eq!(first.len(), MAX_CLEANUP_BEATS);
    assert_eq!(first[0], LogicalTime(0));
    for pair in first.windows(2) {
        assert!(pair[1].0 - pair[0].0 < 5, "{pair:?}");
    }
    let second = match scheduler.observe_time(LogicalTime(20_000)).unwrap() {
        Liveness::Cleaning { beats } => beats,
        other => panic!("expected the continued span, got {other:?}"),
    };
    assert_eq!(second.last().copied(), Some(LogicalTime(20_000)));
    assert!(second[0].0 - first.last().unwrap().0 < 5);
    assert_eq!(scheduler.held("a").len(), 1);
}

#[test]
fn liveness_names_the_same_waiter_admission_tries_first() {
    let mut scheduler = session(
        vec![unit("a", 1, &[("cpu", 1)]), unit("b", 100, &[("gpu", 1)])],
        &[("cpu", 0), ("gpu", 0)],
    );
    match scheduler.admit_next(LogicalTime(1)).unwrap() {
        Admit::Waiting { unit, kind, .. } => {
            assert_eq!(unit, "b");
            assert_eq!(kind, "gpu");
        }
        other => panic!("expected the higher-value wait, got {other:?}"),
    }
    match scheduler.observe_time(LogicalTime(1)).unwrap() {
        Liveness::Waiting { unit, kind, .. } => {
            assert_eq!(unit, "b");
            assert_eq!(kind, "gpu");
        }
        other => panic!("expected the same waiter, got {other:?}"),
    }
}

#[test]
fn a_refused_control_does_not_commit_logical_time() {
    let mut scheduler = session(vec![unit("a", 1, &[("disk", 1)])], &[("disk", 1)]);
    scheduler.admit_next(LogicalTime(0)).unwrap();
    scheduler
        .submit(ControlIntent::Stop, LogicalTime(1))
        .unwrap();
    assert_eq!(
        scheduler.submit(ControlIntent::Drain, LogicalTime(9)),
        Err(ScheduleError::Refused("stopped"))
    );
    match scheduler.observe_time(LogicalTime(3)).unwrap() {
        Liveness::Cleaning { beats } => {
            assert_eq!(beats.first().copied(), Some(LogicalTime(1)));
            for pair in beats.windows(2) {
                assert!(pair[1].0 - pair[0].0 < 5, "{beats:?}");
            }
        }
        other => panic!("expected cleanup after the refused drain, got {other:?}"),
    }
}

#[test]
fn failed_outcome_is_not_retried() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker = CountingWorker {
        calls: Arc::clone(&calls),
        view: WorkerView::Failed,
    };
    let mut scheduler = Session::new(
        vec![unit("a", 1, &[])],
        BTreeSet::from(["run".to_owned()]),
        BTreeMap::new(),
    )
    .unwrap();
    scheduler.admit_next(LogicalTime(0)).unwrap();
    let view = worker.outcome("a");
    assert_eq!(
        scheduler.deliver_outcome("a", view).unwrap(),
        WorkerView::Failed
    );
    assert!(scheduler.deliver_outcome("a", view).is_err());
    assert_eq!(scheduler.failure_set(10).unwrap().failures.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn blocked_worker_view_changes_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker = CountingWorker {
        calls: Arc::clone(&calls),
        view: WorkerView::Blocked,
    };
    let mut scheduler = Session::new(
        vec![unit("a", 1, &[])],
        BTreeSet::from(["run".to_owned()]),
        BTreeMap::new(),
    )
    .unwrap();
    scheduler.admit_next(LogicalTime(0)).unwrap();
    let view = worker.outcome("a");
    assert_eq!(
        scheduler.deliver_outcome("a", view).unwrap(),
        WorkerView::Blocked
    );
    assert!(scheduler.failure_set(10).unwrap().failures.is_empty());
    assert_eq!(scheduler.work_len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn no_change_wake_admits_nothing() {
    let mut scheduler = session(vec![], &[]);
    let live = scheduler
        .wake(WakeEvent::ResourceOffer {
            kind: "cpu".to_owned(),
            amount: 0,
        })
        .unwrap();
    assert_eq!(
        live,
        Liveness::Quiescent {
            wake_on: vec![
                WakeCause::ResourceOffer,
                WakeCause::Control,
                WakeCause::FactRefresh
            ],
        }
    );
    assert_eq!(scheduler.pool("cpu"), 0);
    assert_eq!(scheduler.work_len(), 0);
    scheduler
        .wake(WakeEvent::ResourceOffer {
            kind: "cpu".to_owned(),
            amount: 2,
        })
        .unwrap();
    assert_eq!(scheduler.pool("cpu"), 2);
    assert_eq!(scheduler.work_len(), 0);
    assert_eq!(scheduler.admit_next(LogicalTime(0)).unwrap(), Admit::Idle);
}
