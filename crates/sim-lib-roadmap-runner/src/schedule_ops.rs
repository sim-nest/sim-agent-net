// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    CLEANUP_BEAT, ControlEntry, ControlIntent, Lease, Liveness, LogicalTime, MAX_CLEANUP_BEATS,
    Mode, STALL_AFTER, Session, WakeCause, WorkerView,
};
use crate::schedule::{ProofOutcome, ProofUnit, ScheduleError, legal_order};

impl Session {
    pub(super) fn liveness_at(&mut self, now: LogicalTime) -> Result<Liveness, ScheduleError> {
        if self.mode == Mode::Stopping {
            let beats = self.cleanup_beats(now.0);
            if self.leases.is_empty() {
                self.mode = Mode::Stopped;
                return Ok(Liveness::Stopped {
                    modeled_resources_closed: true,
                });
            }
            return Ok(Liveness::Cleaning { beats });
        }
        if self.mode == Mode::Stopped {
            return Ok(Liveness::Stopped {
                modeled_resources_closed: self.leases.is_empty(),
            });
        }
        if self.mode == Mode::Paused {
            return Ok(Liveness::Quiescent {
                wake_on: vec![WakeCause::Control],
            });
        }
        if let Some(unit) = self.stalled_unit(now.0) {
            let since = LogicalTime(self.last_beat[&unit]);
            return Ok(Liveness::Stalled { unit, since });
        }
        if let Some(wait) = self.next_wait(now)? {
            return Ok(wait);
        }
        if self.work.is_empty() && self.ready_ids()?.is_empty() {
            return Ok(Liveness::Quiescent {
                wake_on: wake_causes(),
            });
        }
        Ok(Liveness::Progressing { beat: now })
    }

    /// The unit the next admission tries first when that unit cannot be granted.
    ///
    /// A candidate that already fits is not reported from an older shortage.
    fn next_wait(&self, now: LogicalTime) -> Result<Option<Liveness>, ScheduleError> {
        let order = legal_order(
            &self.units,
            &self.outcomes,
            &self.capabilities,
            &self.cancelled,
        )?;
        for id in order {
            if self.work.iter().any(|item| item == &id) {
                continue;
            }
            let Some(kind) = self.shortage(&id)? else {
                return Ok(None);
            };
            let since = self
                .waiting
                .get(&id)
                .map(|(_, recorded)| *recorded)
                .unwrap_or(now);
            return Ok(Some(Liveness::Waiting {
                unit: id,
                kind,
                since,
            }));
        }
        Ok(None)
    }

    fn shortage(&self, id: &str) -> Result<Option<String>, ScheduleError> {
        let mut spent: BTreeMap<&str, u64> = BTreeMap::new();
        for ask in &self.unit(id)?.resources {
            let already = spent.get(ask.kind.as_str()).copied().unwrap_or(0);
            let available = self.pool(&ask.kind).saturating_sub(already);
            if ask.amount > available {
                return Ok(Some(ask.kind.clone()));
            }
            *spent.entry(ask.kind.as_str()).or_insert(0) += ask.amount;
        }
        Ok(None)
    }

    pub(super) fn ready_ids(&self) -> Result<Vec<String>, ScheduleError> {
        let mut ready = legal_order(
            &self.units,
            &self.outcomes,
            &self.capabilities,
            &self.cancelled,
        )?;
        ready.retain(|id| !self.work.iter().any(|item| item == id));
        Ok(ready)
    }

    pub(super) fn apply_stop(&mut self, now: LogicalTime) -> Result<(), ScheduleError> {
        if matches!(self.mode, Mode::Stopping | Mode::Stopped) {
            return Ok(());
        }
        self.leave_pause(now);
        self.work.drain(..).for_each(|id| {
            self.cancelled.insert(id);
        });
        self.last_beat.clear();
        self.resume_to = Mode::Running;
        self.mode = if self.leases.is_empty() {
            Mode::Stopped
        } else {
            self.cleanup_origin = Some(now.0);
            Mode::Stopping
        };
        self.push_control(ControlIntent::Stop, now);
        Ok(())
    }

    pub(super) fn apply_drain(&mut self, now: LogicalTime) -> Result<(), ScheduleError> {
        if self.mode == Mode::Draining {
            return Ok(());
        }
        if matches!(self.mode, Mode::Stopping | Mode::Stopped) {
            return Err(ScheduleError::Refused("stopped"));
        }
        self.leave_pause(now);
        self.resume_to = Mode::Draining;
        self.mode = Mode::Draining;
        self.push_control(ControlIntent::Drain, now);
        Ok(())
    }

    pub(super) fn apply_pause(&mut self, now: LogicalTime) -> Result<(), ScheduleError> {
        if self.mode == Mode::Paused {
            return Ok(());
        }
        if matches!(self.mode, Mode::Stopping | Mode::Stopped) {
            return Err(ScheduleError::Refused("stopped"));
        }
        self.resume_to = self.mode;
        self.paused_at = Some(now);
        self.mode = Mode::Paused;
        self.push_control(ControlIntent::Pause, now);
        Ok(())
    }

    pub(super) fn apply_resume(
        &mut self,
        now: LogicalTime,
    ) -> Result<(BTreeSet<String>, BTreeSet<String>), ScheduleError> {
        if self.mode != Mode::Paused {
            return Err(ScheduleError::Refused("not-paused"));
        }
        self.leave_pause(now);
        self.mode = self.resume_to;
        self.push_control(ControlIntent::Resume, now);
        Ok(self.classify_facts(now))
    }

    pub(super) fn leave_pause(&mut self, now: LogicalTime) {
        let Some(paused_at) = self.paused_at.take() else {
            return;
        };
        let delta = now.0.saturating_sub(paused_at.0);
        for beat in self.last_beat.values_mut() {
            *beat = beat.saturating_add(delta);
        }
    }

    pub(super) fn classify_facts(&self, now: LogicalTime) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut expired = BTreeSet::new();
        let mut retained = BTreeSet::new();
        for (id, fact) in &self.facts {
            if fact.observed_at.0.saturating_add(fact.fresh_for) <= now.0 {
                expired.insert(id.clone());
            } else {
                retained.insert(id.clone());
            }
        }
        (expired, retained)
    }

    pub(super) fn cleanup_beats(&mut self, now: u64) -> Vec<LogicalTime> {
        let mut beats = Vec::new();
        let mut cursor = if let Some(tick) = self.last_cleanup {
            tick
        } else {
            let origin = self.cleanup_origin.unwrap_or(now).min(now);
            beats.push(LogicalTime(origin));
            origin
        };
        while cursor < now && beats.len() < MAX_CLEANUP_BEATS {
            let step = cursor.saturating_add(CLEANUP_BEAT);
            let next = if step >= now { now } else { step };
            if next <= cursor {
                break;
            }
            beats.push(LogicalTime(next));
            cursor = next;
        }
        self.last_cleanup = Some(cursor);
        beats
    }

    pub(super) fn stalled_unit(&self, now: u64) -> Option<String> {
        self.work
            .iter()
            .filter(|id| {
                now.saturating_sub(self.last_beat.get(*id).copied().unwrap_or(0)) >= STALL_AFTER
            })
            .min()
            .cloned()
    }

    pub(super) fn try_grant(&mut self, id: &str) -> Result<Option<String>, ScheduleError> {
        let asks = self.unit(id)?.resources.clone();
        let mut taken = Vec::new();
        for ask in asks {
            let left = self.pool(&ask.kind);
            if ask.amount > left {
                self.restore(&taken);
                return Ok(Some(ask.kind));
            }
            *self.pools.entry(ask.kind.clone()).or_insert(0) -= ask.amount;
            taken.push((ask.kind, ask.amount));
        }
        for (kind, amount) in taken {
            let seq = self.lease_seq;
            self.lease_seq += 1;
            let lease_id = format!("lease/{seq}");
            self.leases.insert(
                lease_id.clone(),
                Lease {
                    id: lease_id,
                    holder: id.to_owned(),
                    kind,
                    amount,
                },
            );
        }
        Ok(None)
    }

    pub(super) fn restore(&mut self, taken: &[(String, u64)]) {
        for (kind, amount) in taken {
            *self.pools.entry(kind.clone()).or_insert(0) += amount;
        }
    }

    pub(super) fn store_outcome(
        &mut self,
        id: &str,
        outcome: ProofOutcome,
    ) -> Result<(), ScheduleError> {
        let Some(pos) = self.work.iter().position(|item| item == id) else {
            return Err(ScheduleError::Refused("not-admitted"));
        };
        self.work.remove(pos);
        self.release_holder(id);
        self.last_beat.remove(id);
        self.outcomes.insert(id.to_owned(), outcome);
        Ok(())
    }

    pub(super) fn release_holder(&mut self, id: &str) {
        let ids: Vec<String> = self
            .leases
            .iter()
            .filter(|(_, lease)| lease.holder == id)
            .map(|(lease_id, _)| lease_id.clone())
            .collect();
        for lease_id in ids {
            if let Some(lease) = self.leases.remove(&lease_id) {
                *self.pools.entry(lease.kind).or_insert(0) += lease.amount;
            }
        }
    }

    pub(super) fn release_all(&mut self) {
        let holders: Vec<String> = self
            .leases
            .values()
            .map(|lease| lease.holder.clone())
            .collect();
        let mut seen = BTreeSet::new();
        for holder in holders {
            if seen.insert(holder.clone()) {
                self.release_holder(&holder);
            }
        }
    }

    pub(super) fn unit(&self, id: &str) -> Result<&ProofUnit, ScheduleError> {
        self.units
            .iter()
            .find(|unit| unit.id == id)
            .ok_or_else(|| ScheduleError::UnknownUnit(id.to_owned()))
    }

    pub(super) fn push_control(&mut self, intent: ControlIntent, at: LogicalTime) {
        self.control_log.push(ControlEntry { intent, at });
    }

    pub(super) fn advance_now(&mut self, now: LogicalTime) -> Result<(), ScheduleError> {
        if now < self.now {
            return Err(ScheduleError::TimeRewind);
        }
        self.now = now;
        Ok(())
    }

    pub(super) fn admit_block(&self) -> Option<&'static str> {
        match self.mode {
            Mode::Running => None,
            Mode::Draining => Some("draining"),
            Mode::Paused => Some("paused"),
            Mode::Stopping | Mode::Stopped => Some("stopped"),
        }
    }

    /// Record a view the caller already obtained. This does not call a worker.
    pub fn deliver_outcome(
        &mut self,
        id: &str,
        view: WorkerView,
    ) -> Result<WorkerView, ScheduleError> {
        self.ensure_outcome_allowed()?;
        if !self.work.iter().any(|item| item == id) {
            return Err(ScheduleError::Refused("not-admitted"));
        }
        match view {
            WorkerView::Blocked => Ok(WorkerView::Blocked),
            WorkerView::Passed => {
                self.store_outcome(id, ProofOutcome::Passed)?;
                Ok(WorkerView::Passed)
            }
            WorkerView::Failed => {
                self.store_outcome(id, ProofOutcome::Failed)?;
                Ok(WorkerView::Failed)
            }
        }
    }

    pub(super) fn ensure_outcome_allowed(&self) -> Result<(), ScheduleError> {
        match self.mode {
            Mode::Running | Mode::Draining => Ok(()),
            Mode::Paused => Err(ScheduleError::Refused("paused")),
            Mode::Stopping | Mode::Stopped => Err(ScheduleError::Refused("stopped")),
        }
    }
}

fn wake_causes() -> Vec<WakeCause> {
    vec![
        WakeCause::ResourceOffer,
        WakeCause::Control,
        WakeCause::FactRefresh,
    ]
}
