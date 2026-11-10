// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Independent control, finite resource offers, and logical time.
//!
//! Stop, drain, pause, and resume are recorded on a control log. Applying an
//! entry does not read worker outcomes. A wait releases only the leases taken
//! for the waiting unit and does not occupy the next unit's slot. Cleanup
//! beats are logical facts. Closing modeled resources does not establish
//! process death.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::schedule::{
    FailureFrontier, ProofOutcome, ProofUnit, ScheduleError, check_id, failure_frontier,
    legal_order,
};

/// Logical ticks. These are not wall-clock milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalTime(pub u64);

/// Silence at or above this many ticks is a stall.
pub const STALL_AFTER: u64 = 30;
/// Cleanup observations are split so consecutive beats stay inside this gap.
pub const CLEANUP_BEAT: u64 = 4;
/// Admitted units that have not finished. Further admission refuses.
pub const MAX_ADMITTED: usize = 64;
const MAX_CLEANUP_BEATS: usize = 4096;

/// Control carried on the independent log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlIntent {
    /// Cancel admitted work and keep the cleanup guardian.
    Stop,
    /// Refuse new admission and let admitted work finish.
    Drain,
    /// Preserve state and perform no work.
    Pause,
    /// Leave pause and name facts that must be observed again.
    Resume,
}

/// Session mode after a control entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// New legal units may be admitted.
    Running,
    /// Admitted units may finish. New admission is refused.
    Draining,
    /// No admission, report, or collection is accepted.
    Paused,
    /// Admitted work is cancelled and modeled leases are still open.
    Stopping,
    /// Modeled leases are closed.
    Stopped,
}

/// One recorded control entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlEntry {
    /// Intent that was applied.
    pub intent: ControlIntent,
    /// Logical time of the entry.
    pub at: LogicalTime,
}

/// Acknowledgement produced without consulting a worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlAck {
    /// Intent that was applied.
    pub intent: ControlIntent,
    /// Logical time of the acknowledgement.
    pub at: LogicalTime,
    /// Mode after the entry.
    pub mode: Mode,
    /// Facts whose freshness ended at resume.
    pub expired: BTreeSet<String>,
    /// Facts that were still fresh at resume.
    pub retained: BTreeSet<String>,
}

/// What one explicit admission attempt did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admit {
    /// The unit holds its leases and occupies one work slot.
    Admitted {
        /// Admitted unit.
        unit: String,
    },
    /// No legal unit could be granted a slot.
    Waiting {
        /// Highest-value unit that is still waiting.
        unit: String,
        /// Resource that was short.
        kind: String,
        /// Logical time of the first wait for this unit.
        since: LogicalTime,
    },
    /// Nothing legal remains.
    Idle,
    /// The mode refuses admission.
    Refused(&'static str),
}

/// Model of a worker result delivered by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerView {
    /// The worker reported a pass.
    Passed,
    /// The worker reported a failure.
    Failed,
    /// The caller still has no finished result. Delivering this changes nothing.
    Blocked,
}

/// Model worker. The session does not call this trait.
///
/// The caller runs `outcome` with no borrow of [`Session`], then delivers the
/// view. A call that does not return never reaches delivery, and control
/// methods stay available on the session.
pub trait ModelWorker {
    /// Return the worker's current view of one unit.
    fn outcome(&self, id: &str) -> WorkerView;
}

/// One held resource lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    /// Stable lease id.
    pub id: String,
    /// Unit that holds the lease.
    pub holder: String,
    /// Resource class.
    pub kind: String,
    /// Amount charged to the pool.
    pub amount: u64,
}

/// Causes a quiescent session can wake for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeCause {
    /// A resource amount changed.
    ResourceOffer,
    /// A control entry arrived.
    Control,
    /// A fact was refreshed.
    FactRefresh,
}

/// One wake event supplied by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeEvent {
    /// Offer a finite amount of one resource class.
    ResourceOffer {
        /// Resource class.
        kind: String,
        /// Amount available after the offer.
        amount: u64,
    },
    /// Apply one control intent.
    Control(ControlIntent),
    /// Record a fact observation.
    FactRefresh {
        /// Fact id.
        id: String,
        /// Logical time of the observation.
        observed_at: LogicalTime,
        /// Ticks the observation stays fresh.
        fresh_for: u64,
    },
}

/// Liveness classified from logical time and session state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Liveness {
    /// Admitted work still has a fresh beat.
    Progressing {
        /// Logical time of this beat.
        beat: LogicalTime,
    },
    /// A unit is waiting on a resource.
    Waiting {
        /// Waiting unit.
        unit: String,
        /// Short resource.
        kind: String,
        /// First logical time of the wait.
        since: LogicalTime,
    },
    /// A beat is missing. This is not a failure, retry, or kill.
    Stalled {
        /// Unit whose beat is missing.
        unit: String,
        /// Logical time of its last beat.
        since: LogicalTime,
    },
    /// Cleanup guardian beats while modeled leases remain.
    Cleaning {
        /// Beats emitted by this observation, with no gap of five ticks.
        beats: Vec<LogicalTime>,
    },
    /// No admitted work and no legal unit.
    Quiescent {
        /// Causes that can wake the session.
        wake_on: Vec<WakeCause>,
    },
    /// Stop completed for modeled resources.
    Stopped {
        /// Every modeled lease is closed.
        modeled_resources_closed: bool,
    },
}

struct Fact {
    observed_at: LogicalTime,
    fresh_for: u64,
}

/// Proof schedule, resource table, and independent control log.
pub struct Session {
    units: Vec<ProofUnit>,
    capabilities: BTreeSet<String>,
    outcomes: BTreeMap<String, ProofOutcome>,
    pools: BTreeMap<String, u64>,
    leases: BTreeMap<String, Lease>,
    work: VecDeque<String>,
    waiting: BTreeMap<String, (String, LogicalTime)>,
    cancelled: BTreeSet<String>,
    control_log: Vec<ControlEntry>,
    facts: BTreeMap<String, Fact>,
    last_beat: BTreeMap<String, u64>,
    mode: Mode,
    resume_to: Mode,
    now: LogicalTime,
    paused_at: Option<LogicalTime>,
    cleanup_origin: Option<u64>,
    last_cleanup: Option<u64>,
    lease_seq: u64,
}

impl Session {
    /// Build a session. No worker is stored or called.
    pub fn new(
        units: Vec<ProofUnit>,
        capabilities: BTreeSet<String>,
        pools: BTreeMap<String, u64>,
    ) -> Result<Self, ScheduleError> {
        failure_frontier(&units, &BTreeMap::new(), &capabilities, &BTreeSet::new(), 0)?;
        Ok(Self {
            units,
            capabilities,
            outcomes: BTreeMap::new(),
            pools,
            leases: BTreeMap::new(),
            work: VecDeque::new(),
            waiting: BTreeMap::new(),
            cancelled: BTreeSet::new(),
            control_log: Vec::new(),
            facts: BTreeMap::new(),
            last_beat: BTreeMap::new(),
            mode: Mode::Running,
            resume_to: Mode::Running,
            now: LogicalTime(0),
            paused_at: None,
            cleanup_origin: None,
            last_cleanup: None,
            lease_seq: 0,
        })
    }

    /// Current mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Control entries in application order.
    pub fn control_log(&self) -> &[ControlEntry] {
        &self.control_log
    }

    /// Admitted units that have not finished.
    pub fn work_len(&self) -> usize {
        self.work.len()
    }

    /// Amount left in one pool.
    pub fn pool(&self, kind: &str) -> u64 {
        self.pools.get(kind).copied().unwrap_or(0)
    }

    /// Leases held by one unit.
    pub fn held<'a>(&'a self, unit: &str) -> Vec<&'a Lease> {
        self.leases
            .values()
            .filter(|lease| lease.holder == unit)
            .collect()
    }

    /// Modeled closure is not process death.
    pub const fn os_terminated(&self) -> bool {
        false
    }

    /// Frontier over recorded outcomes. Cancelled units are skipped.
    pub fn failure_set(&self, budget: u32) -> Result<FailureFrontier, ScheduleError> {
        failure_frontier(
            &self.units,
            &self.outcomes,
            &self.capabilities,
            &self.cancelled,
            budget,
        )
    }

    /// Record one fact. This does not admit work.
    pub fn insert_fact(
        &mut self,
        id: &str,
        observed_at: LogicalTime,
        fresh_for: u64,
    ) -> Result<(), ScheduleError> {
        check_id(id)?;
        self.facts.insert(
            id.to_owned(),
            Fact {
                observed_at,
                fresh_for,
            },
        );
        Ok(())
    }

    /// Admit the next legal unit whose resources fit. A wait does not take the slot.
    pub fn admit_next(&mut self, now: LogicalTime) -> Result<Admit, ScheduleError> {
        self.advance_now(now)?;
        if let Some(reason) = self.admit_block() {
            return Ok(Admit::Refused(reason));
        }
        let order = legal_order(
            &self.units,
            &self.outcomes,
            &self.capabilities,
            &self.cancelled,
        )?;
        let mut waited: Option<(String, String)> = None;
        for id in order {
            if self.work.iter().any(|item| item == &id) {
                continue;
            }
            if self.work.len() >= MAX_ADMITTED {
                return Ok(Admit::Refused("admission-bound"));
            }
            match self.try_grant(&id)? {
                None => {
                    self.waiting.remove(&id);
                    self.work.push_back(id.clone());
                    self.last_beat.insert(id.clone(), now.0);
                    return Ok(Admit::Admitted { unit: id });
                }
                Some(kind) => {
                    self.waiting
                        .entry(id.clone())
                        .and_modify(|(recorded, _)| *recorded = kind.clone())
                        .or_insert((kind.clone(), now));
                    waited.get_or_insert((id, kind));
                }
            }
        }
        Ok(match waited {
            Some((unit, kind)) => {
                let since = self.waiting[&unit].1;
                Admit::Waiting { unit, kind, since }
            }
            None => Admit::Idle,
        })
    }

    /// Record an explicit outcome for an admitted unit. This does not retry.
    pub fn report(&mut self, id: &str, outcome: ProofOutcome) -> Result<(), ScheduleError> {
        self.ensure_outcome_allowed()?;
        self.store_outcome(id, outcome)
    }

    /// Apply one control intent. The worker is not consulted.
    pub fn submit(
        &mut self,
        intent: ControlIntent,
        now: LogicalTime,
    ) -> Result<ControlAck, ScheduleError> {
        let previous = self.now;
        self.advance_now(now)?;
        let applied = match intent {
            ControlIntent::Stop => self.apply_stop(now),
            ControlIntent::Drain => self.apply_drain(now),
            ControlIntent::Pause => self.apply_pause(now),
            ControlIntent::Resume => self.apply_resume(now).map(|_| ()),
        };
        if let Err(err) = applied {
            self.now = previous;
            return Err(err);
        }
        let (expired, retained) = if intent == ControlIntent::Resume {
            self.classify_facts(now)
        } else {
            (BTreeSet::new(), BTreeSet::new())
        };
        Ok(ControlAck {
            intent,
            at: now,
            mode: self.mode,
            expired,
            retained,
        })
    }

    /// Classify liveness at one logical time. This does not poll or call the worker.
    ///
    /// A refused classification restores the previous logical time so a later
    /// observation can still move forward from the last accepted tick.
    pub fn observe_time(&mut self, now: LogicalTime) -> Result<Liveness, ScheduleError> {
        let previous = self.now;
        self.advance_now(now)?;
        match self.liveness_at(now) {
            Ok(live) => Ok(live),
            Err(err) => {
                self.now = previous;
                Err(err)
            }
        }
    }

    /// Wake on one event. An unchanged offer admits nothing and calls no worker.
    pub fn wake(&mut self, event: WakeEvent) -> Result<Liveness, ScheduleError> {
        match event {
            WakeEvent::ResourceOffer { kind, amount } => {
                check_id(&kind)?;
                if amount > self.pool(&kind) {
                    self.pools.insert(kind, amount);
                }
            }
            WakeEvent::Control(intent) => {
                self.submit(intent, self.now)?;
            }
            WakeEvent::FactRefresh {
                id,
                observed_at,
                fresh_for,
            } => {
                let same = self.facts.get(&id).is_some_and(|fact| {
                    fact.observed_at == observed_at && fact.fresh_for == fresh_for
                });
                if !same {
                    self.insert_fact(&id, observed_at, fresh_for)?;
                }
            }
        }
        self.liveness_at(self.now)
    }

    /// Release every modeled lease and enter the stopped mode.
    pub fn close_modeled_resources(&mut self) -> Result<(), ScheduleError> {
        if !matches!(self.mode, Mode::Stopping | Mode::Stopped) {
            return Err(ScheduleError::Refused("not-stopping"));
        }
        self.release_all();
        self.mode = Mode::Stopped;
        Ok(())
    }
}

#[path = "schedule_ops.rs"]
mod ops;

#[cfg(test)]
#[path = "schedule_session_tests.rs"]
mod schedule_session_tests;

#[cfg(test)]
#[path = "schedule_session_more_tests.rs"]
mod schedule_session_more_tests;
