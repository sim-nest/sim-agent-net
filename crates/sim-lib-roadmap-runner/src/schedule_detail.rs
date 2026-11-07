// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    CLASS_CYCLE, CLASS_MISSING_CAPABILITY, CLASS_MISSING_DEPENDENCY, CLASS_PROOF_FAILED,
    ProofOutcome, ProofUnit, ScheduleError, TypedFailure, check_id, information_value,
};

const HEADER: &str = "frontier-continuation-v1";

pub(super) fn runnable(
    id: &str,
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
    outcomes: &BTreeMap<String, ProofOutcome>,
    skip: &BTreeSet<String>,
) -> Result<bool, ScheduleError> {
    if skip.contains(id) || is_structural(id, units, cyclic, capabilities) {
        return Ok(false);
    }
    deps_passed(id, units, cyclic, capabilities, outcomes, skip)
}

pub(super) fn deps_passed(
    id: &str,
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
    outcomes: &BTreeMap<String, ProofOutcome>,
    skip: &BTreeSet<String>,
) -> Result<bool, ScheduleError> {
    for dep in &units[id].depends_on {
        if !units.contains_key(dep) {
            continue;
        }
        let passed = outcomes.get(dep) == Some(&ProofOutcome::Passed)
            && !skip.contains(dep)
            && !is_structural(dep, units, cyclic, capabilities);
        if !passed {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn failures_of(
    id: &str,
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
    outcomes: &BTreeMap<String, ProofOutcome>,
) -> BTreeSet<TypedFailure> {
    let unit = &units[id];
    let mut out = BTreeSet::new();
    if cyclic.contains(id) {
        out.insert(failure(id, CLASS_CYCLE, ""));
    }
    let mut missing: Vec<&str> = unit
        .depends_on
        .iter()
        .filter(|dep| !units.contains_key(dep.as_str()))
        .map(String::as_str)
        .collect();
    missing.sort_unstable();
    missing.dedup();
    for dep in missing {
        out.insert(failure(id, CLASS_MISSING_DEPENDENCY, dep));
    }
    if !capabilities.contains(&unit.capability) {
        out.insert(failure(id, CLASS_MISSING_CAPABILITY, ""));
    }
    if out.is_empty() && outcomes.get(id) == Some(&ProofOutcome::Failed) {
        out.insert(failure(id, CLASS_PROOF_FAILED, ""));
    }
    out
}

fn failure(unit: &str, class: &'static str, detail: &str) -> TypedFailure {
    TypedFailure {
        unit: unit.to_owned(),
        class,
        detail: detail.to_owned(),
    }
}

pub(super) fn sort_by_value(units: &BTreeMap<String, ProofUnit>, ids: &mut [String]) {
    ids.sort_by(|left, right| {
        information_value(&units[right])
            .cmp(&information_value(&units[left]))
            .then_with(|| left.cmp(right))
    });
}

pub(super) fn is_structural(
    id: &str,
    units: &BTreeMap<String, ProofUnit>,
    cyclic: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
) -> bool {
    let unit = &units[id];
    cyclic.contains(id)
        || unit.depends_on.iter().any(|dep| !units.contains_key(dep))
        || !capabilities.contains(&unit.capability)
}

pub(super) fn index_units(
    units: &[ProofUnit],
) -> Result<BTreeMap<String, ProofUnit>, ScheduleError> {
    let mut indexed = BTreeMap::new();
    for unit in units {
        check_id(&unit.id)?;
        check_id(&unit.capability)?;
        for dep in &unit.depends_on {
            check_id(dep)?;
        }
        for ask in &unit.resources {
            check_id(&ask.kind)?;
            if ask.amount == 0 {
                return Err(ScheduleError::Refused("empty-resource"));
            }
        }
        if indexed.insert(unit.id.clone(), unit.clone()).is_some() {
            return Err(ScheduleError::DuplicateUnit(unit.id.clone()));
        }
    }
    Ok(indexed)
}

pub(super) fn check_names(
    units: &BTreeMap<String, ProofUnit>,
    outcomes: &BTreeMap<String, ProofOutcome>,
    skip: &BTreeSet<String>,
) -> Result<(), ScheduleError> {
    for id in outcomes.keys().chain(skip.iter()) {
        if !units.contains_key(id) {
            return Err(ScheduleError::UnknownUnit(id.clone()));
        }
    }
    if skip.iter().any(|id| outcomes.contains_key(id)) {
        return Err(ScheduleError::Refused("skip-conflict"));
    }
    Ok(())
}

pub(super) fn cyclic_units(units: &BTreeMap<String, ProofUnit>) -> BTreeSet<String> {
    let mut color = BTreeMap::<String, u8>::new();
    let mut stack = Vec::new();
    let mut cyclic = BTreeSet::new();
    for id in units.keys() {
        if color.get(id).copied().unwrap_or(0) == 0 {
            walk(id, units, &mut color, &mut stack, &mut cyclic);
        }
    }
    cyclic
}

fn walk(
    id: &str,
    units: &BTreeMap<String, ProofUnit>,
    color: &mut BTreeMap<String, u8>,
    stack: &mut Vec<String>,
    cyclic: &mut BTreeSet<String>,
) {
    color.insert(id.to_owned(), 1);
    stack.push(id.to_owned());
    for dep in &units[id].depends_on {
        if !units.contains_key(dep) {
            continue;
        }
        match color.get(dep).copied().unwrap_or(0) {
            1 => {
                if let Some(pos) = stack.iter().position(|item| item == dep) {
                    cyclic.extend(stack[pos..].iter().cloned());
                }
            }
            0 => walk(dep, units, color, stack, cyclic),
            _ => {}
        }
    }
    stack.pop();
    color.insert(id.to_owned(), 2);
}

pub(super) struct Decoded {
    pub(super) examined: Vec<String>,
    pub(super) failures: BTreeSet<TypedFailure>,
    pub(super) remaining: BTreeSet<String>,
}

pub(super) fn encode(
    examined: &[String],
    failures: &BTreeSet<TypedFailure>,
    remaining: &BTreeSet<String>,
) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    push_ids(&mut out, "examined", examined);
    out.push_str(&format!("failures {}\n", failures.len()));
    for failure in failures {
        let detail = if failure.detail.is_empty() {
            "*"
        } else {
            failure.detail.as_str()
        };
        out.push_str(&format!("{} {} {}\n", failure.unit, failure.class, detail));
    }
    let remaining: Vec<String> = remaining.iter().cloned().collect();
    push_ids(&mut out, "remaining", &remaining);
    out
}

fn push_ids(out: &mut String, label: &str, ids: &[String]) {
    out.push_str(&format!("{label} {}\n", ids.len()));
    for id in ids {
        out.push_str(id);
        out.push('\n');
    }
}

pub(super) fn decode_continuation(text: &str) -> Result<Decoded, ScheduleError> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    let examined = read_ids(&mut lines, "examined")?;
    let failures = read_failures(&mut lines)?;
    let remaining_list = read_ids(&mut lines, "remaining")?;
    if lines.next().is_some() {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    Ok(Decoded {
        examined,
        failures,
        remaining: remaining_list.into_iter().collect(),
    })
}

fn read_ids<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<Vec<String>, ScheduleError> {
    let count = section_count(lines, label)?;
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        let id = lines
            .next()
            .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
        check_id(id)?;
        ids.push(id.to_owned());
    }
    Ok(ids)
}

fn read_failures<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<TypedFailure>, ScheduleError> {
    let count = section_count(lines, "failures")?;
    let mut failures = BTreeSet::new();
    for _ in 0..count {
        let line = lines
            .next()
            .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
        let mut parts = line.split(' ');
        let unit = parts
            .next()
            .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
        let class = parts
            .next()
            .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
        let detail = parts
            .next()
            .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
        if parts.next().is_some() {
            return Err(ScheduleError::Refused("continuation-mismatch"));
        }
        check_id(unit)?;
        let class = class_token(class)?;
        let detail = if detail == "*" {
            String::new()
        } else {
            check_id(detail)?;
            detail.to_owned()
        };
        failures.insert(TypedFailure {
            unit: unit.to_owned(),
            class,
            detail,
        });
    }
    if failures.len() != count {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    Ok(failures)
}

fn section_count<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<usize, ScheduleError> {
    let line = lines
        .next()
        .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
    let mut parts = line.split(' ');
    if parts.next() != Some(label) {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    let count = parts
        .next()
        .ok_or(ScheduleError::Refused("continuation-mismatch"))?;
    if parts.next().is_some() {
        return Err(ScheduleError::Refused("continuation-mismatch"));
    }
    count
        .parse()
        .map_err(|_| ScheduleError::Refused("continuation-mismatch"))
}

fn class_token(class: &str) -> Result<&'static str, ScheduleError> {
    match class {
        CLASS_CYCLE => Ok(CLASS_CYCLE),
        CLASS_MISSING_DEPENDENCY => Ok(CLASS_MISSING_DEPENDENCY),
        CLASS_MISSING_CAPABILITY => Ok(CLASS_MISSING_CAPABILITY),
        CLASS_PROOF_FAILED => Ok(CLASS_PROOF_FAILED),
        _ => Err(ScheduleError::Refused("continuation-mismatch")),
    }
}
