// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Complete proof frontiers and the legal order of runnable units.
//!
//! Dependency and capability legality are decided before information-value
//! order. A partial budget returns a continuation that still names every
//! unexamined unit. Learned cost is diagnostic: a zero cost adds no ratio.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

#[path = "schedule_detail.rs"]
mod detail;
use detail::{
    Decoded, check_names, cyclic_units, decode_continuation, deps_passed, encode, failures_of,
    index_units, is_structural, runnable, sort_by_value,
};

/// Dependency cycle among declared units.
pub const CLASS_CYCLE: &str = "dependency-cycle";
/// A dependency name is not a declared unit.
pub const CLASS_MISSING_DEPENDENCY: &str = "missing-dependency";
/// The unit's capability is not in the offered set.
pub const CLASS_MISSING_CAPABILITY: &str = "missing-capability";
/// A legal unit finished with a failed proof outcome.
pub const CLASS_PROOF_FAILED: &str = "proof-failed";

/// Why frontier construction or continuation refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScheduleError {
    /// Two units share one id.
    #[error("duplicate schedule unit: {0}")]
    DuplicateUnit(String),
    /// The id cannot be stored in a continuation line.
    #[error("invalid schedule id: {0}")]
    InvalidId(String),
    /// An outcome or skip names a unit that was not declared.
    #[error("unknown schedule unit: {0}")]
    UnknownUnit(String),
    /// Logical time moved backwards.
    #[error("logical time moved backwards")]
    TimeRewind,
    /// The call is not legal in the current state.
    #[error("schedule refused: {0}")]
    Refused(&'static str),
}

/// One finite resource request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAsk {
    /// Resource class offered by the model provider.
    pub kind: String,
    /// Exact amount required before admission.
    pub amount: u64,
}

/// One proof unit and the numbers used to order it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofUnit {
    /// Stable unit id.
    pub id: String,
    /// Units that must have passed before this one is legal.
    pub depends_on: Vec<String>,
    /// Capability the model worker must offer.
    pub capability: String,
    /// Expected invalidation mass.
    pub invalidation_mass: u64,
    /// Diagnostic learned cost. Zero adds no ratio term.
    pub learned_cost: u64,
    /// Critical-path gain added after the ratio.
    pub critical_path_gain: u64,
    /// Novelty gain added after the ratio.
    pub novelty_gain: u64,
    /// Finite resources required to admit the unit.
    pub resources: Vec<ResourceAsk>,
}

/// Caller-supplied proof result. Absence is unresolved, not failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofOutcome {
    /// The proof passed.
    Passed,
    /// The proof failed.
    Failed,
}

/// One classified failure. The set is independent of report order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypedFailure {
    /// Unit the failure belongs to.
    pub unit: String,
    /// Stable class token.
    pub class: &'static str,
    /// Missing dependency id, or empty.
    pub detail: String,
}

/// Lossless remainder of a frontier that stopped inside its budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Continuation {
    /// Canonical text that still contains every unexamined id.
    pub text: String,
    /// Units already classified, in examination order.
    pub examined: Vec<String>,
    /// Failures named by the examined prefix.
    pub failures: BTreeSet<TypedFailure>,
    /// Classified-but-unexamined units and runnable units with no outcome.
    pub remaining: BTreeSet<String>,
}

/// Failure set for one budget, plus the conservative floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureFrontier {
    /// Failures inside the examined prefix.
    pub failures: BTreeSet<TypedFailure>,
    /// Present while any classifiable or unresolved unit remains.
    pub continuation: Option<Continuation>,
    /// Every declared unit that has not passed and was not skipped.
    pub floor: BTreeSet<String>,
}

/// Integer information value. A zero learned cost contributes no ratio.
pub fn information_value(unit: &ProofUnit) -> u128 {
    let ratio = if unit.learned_cost == 0 {
        0
    } else {
        u128::from(unit.invalidation_mass) / u128::from(unit.learned_cost)
    };
    ratio + u128::from(unit.critical_path_gain) + u128::from(unit.novelty_gain)
}

/// Runnable units that have no outcome yet, highest information value first.
pub fn legal_order(
    units: &[ProofUnit],
    outcomes: &BTreeMap<String, ProofOutcome>,
    capabilities: &BTreeSet<String>,
    skip: &BTreeSet<String>,
) -> Result<Vec<String>, ScheduleError> {
    let indexed = index_units(units)?;
    check_names(&indexed, outcomes, skip)?;
    let cyclic = cyclic_units(&indexed);
    let mut ready = Vec::new();
    for id in indexed.keys() {
        if runnable(id, &indexed, &cyclic, capabilities, outcomes, skip)?
            && !outcomes.contains_key(id)
        {
            ready.push(id.clone());
        }
    }
    sort_by_value(&indexed, &mut ready);
    Ok(ready)
}

/// Classify the whole declared set, or the budget's prefix, in legal order.
pub fn failure_frontier(
    units: &[ProofUnit],
    outcomes: &BTreeMap<String, ProofOutcome>,
    capabilities: &BTreeSet<String>,
    skip: &BTreeSet<String>,
    budget: u32,
) -> Result<FailureFrontier, ScheduleError> {
    project(units, outcomes, capabilities, skip, budget, None)
}

/// Continue a frontier from a continuation minted by [`failure_frontier`].
pub fn resume_frontier(
    units: &[ProofUnit],
    outcomes: &BTreeMap<String, ProofOutcome>,
    capabilities: &BTreeSet<String>,
    skip: &BTreeSet<String>,
    token: &str,
    budget: u32,
) -> Result<FailureFrontier, ScheduleError> {
    let decoded = decode_continuation(token)?;
    project(units, outcomes, capabilities, skip, budget, Some(&decoded))
}

pub(crate) fn check_id(id: &str) -> Result<(), ScheduleError> {
    let ok = !id.is_empty()
        && id != "*"
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'));
    if ok {
        Ok(())
    } else {
        Err(ScheduleError::InvalidId(id.to_owned()))
    }
}

fn project(
    units: &[ProofUnit],
    outcomes: &BTreeMap<String, ProofOutcome>,
    capabilities: &BTreeSet<String>,
    skip: &BTreeSet<String>,
    budget: u32,
    token: Option<&Decoded>,
) -> Result<FailureFrontier, ScheduleError> {
    let indexed = index_units(units)?;
    check_names(&indexed, outcomes, skip)?;
    let cyclic = cyclic_units(&indexed);
    let plan = examination_plan(&indexed, &cyclic, capabilities, outcomes, skip)?;
    let start = match token {
        Some(decoded) => {
            check_token(decoded, &plan)?;
            decoded.examined.len()
        }
        None => 0,
    };
    let end = start
        .saturating_add(budget as usize)
        .min(plan.sequence.len());
    let examined = plan.sequence[start..end].to_vec();
    let mut failures = token
        .map(|decoded| decoded.failures.clone())
        .unwrap_or_default();
    for id in &examined {
        failures.extend(plan.failures[id].iter().cloned());
    }
    let mut remaining = plan.unresolved.clone();
    remaining.extend(plan.sequence[end..].iter().cloned());
    let continuation = if remaining.is_empty() {
        None
    } else {
        Some(Continuation {
            text: encode(&plan.sequence[..end], &failures, &remaining),
            examined: plan.sequence[..end].to_vec(),
            failures: failures.clone(),
            remaining,
        })
    };
    Ok(FailureFrontier {
        failures,
        continuation,
        floor: floor_of(&indexed, &cyclic, capabilities, outcomes, skip)?,
    })
}

struct Plan {
    sequence: Vec<String>,
    unresolved: BTreeSet<String>,
    failures: BTreeMap<String, BTreeSet<TypedFailure>>,
}

fn examination_plan(
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
    outcomes: &BTreeMap<String, ProofOutcome>,
    skip: &BTreeSet<String>,
) -> Result<Plan, ScheduleError> {
    let mut structural = Vec::new();
    let mut failed = Vec::new();
    let mut unresolved = BTreeSet::new();
    for id in units.keys() {
        if skip.contains(id) {
            continue;
        }
        if is_structural(id, units, cyclic, capabilities) {
            structural.push(id.clone());
            continue;
        }
        if !deps_passed(id, units, cyclic, capabilities, outcomes, skip)? {
            continue;
        }
        match outcomes.get(id) {
            Some(ProofOutcome::Failed) => failed.push(id.clone()),
            Some(ProofOutcome::Passed) => {}
            None => {
                unresolved.insert(id.clone());
            }
        }
    }
    structural.sort();
    sort_by_value(units, &mut failed);
    let mut sequence = structural;
    sequence.append(&mut failed);
    let mut failures = BTreeMap::new();
    for id in &sequence {
        failures.insert(
            id.clone(),
            failures_of(id, units, cyclic, capabilities, outcomes),
        );
    }
    Ok(Plan {
        sequence,
        unresolved,
        failures,
    })
}

fn floor_of(
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
    outcomes: &BTreeMap<String, ProofOutcome>,
    skip: &BTreeSet<String>,
) -> Result<BTreeSet<String>, ScheduleError> {
    let mut floor = BTreeSet::new();
    for id in units.keys() {
        if skip.contains(id) {
            continue;
        }
        let passed = outcomes.get(id) == Some(&ProofOutcome::Passed)
            && !is_structural(id, units, cyclic, capabilities);
        if !passed {
            floor.insert(id.clone());
        }
    }
    Ok(floor)
}

fn check_token(decoded: &Decoded, plan: &Plan) -> Result<(), ScheduleError> {
    let n = decoded.examined.len();
    if plan.sequence.get(..n) != Some(decoded.examined.as_slice()) {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    let mut expect = BTreeSet::new();
    for id in &decoded.examined {
        expect.extend(plan.failures[id].iter().cloned());
    }
    if expect != decoded.failures {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    let mut remaining = plan.unresolved.clone();
    remaining.extend(plan.sequence[n..].iter().cloned());
    if remaining != decoded.remaining {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod schedule_tests;
