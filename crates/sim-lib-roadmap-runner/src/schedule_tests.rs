// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    CLASS_CYCLE, CLASS_MISSING_CAPABILITY, CLASS_MISSING_DEPENDENCY, CLASS_PROOF_FAILED,
    ProofOutcome, ProofUnit, ResourceAsk, ScheduleError, failure_frontier, legal_order,
    resume_frontier,
};

fn unit(
    id: &str,
    depends_on: &[&str],
    capability: &str,
    mass: u64,
    cost: u64,
    critical: u64,
    novelty: u64,
) -> ProofUnit {
    ProofUnit {
        id: id.to_owned(),
        depends_on: depends_on.iter().map(|dep| (*dep).to_owned()).collect(),
        capability: capability.to_owned(),
        invalidation_mass: mass,
        learned_cost: cost,
        critical_path_gain: critical,
        novelty_gain: novelty,
        resources: Vec::new(),
    }
}

fn caps() -> BTreeSet<String> {
    BTreeSet::from(["run".to_owned()])
}

fn outcomes(pairs: &[(&str, ProofOutcome)]) -> BTreeMap<String, ProofOutcome> {
    pairs
        .iter()
        .map(|(id, outcome)| ((*id).to_owned(), *outcome))
        .collect()
}

fn classes(frontier: &super::FailureFrontier) -> BTreeSet<(String, &'static str)> {
    frontier
        .failures
        .iter()
        .map(|failure| (failure.unit.clone(), failure.class))
        .collect()
}

#[test]
fn failure_order_does_not_change_the_set() {
    let failed = ProofOutcome::Failed;
    let first_units = vec![
        unit("b", &[], "run", 0, 0, 0, 0),
        unit("a", &[], "run", 0, 0, 0, 0),
        unit("c", &[], "run", 0, 0, 0, 0),
    ];
    let mut reversed = first_units.clone();
    reversed.reverse();
    let skip = BTreeSet::new();
    let forward_map = outcomes(&[("c", failed), ("a", failed), ("b", failed)]);
    let reverse_map = outcomes(&[("b", failed), ("a", failed), ("c", failed)]);
    let forward = failure_frontier(&first_units, &forward_map, &caps(), &skip, 10).unwrap();
    let backward = failure_frontier(&reversed, &reverse_map, &caps(), &skip, 10).unwrap();
    let partial_forward = failure_frontier(&first_units, &forward_map, &caps(), &skip, 1).unwrap();
    let partial_backward = failure_frontier(&reversed, &reverse_map, &caps(), &skip, 1).unwrap();
    assert_eq!(forward.failures, backward.failures);
    assert_eq!(partial_forward.failures, partial_backward.failures);
    assert_eq!(
        classes(&partial_forward),
        BTreeSet::from([("a".to_owned(), CLASS_PROOF_FAILED)])
    );
    assert!(forward.continuation.is_none());
}

#[test]
fn partial_budget_continuation_resumes_to_the_same_set() {
    let units = vec![
        unit("a", &[], "run", 0, 0, 0, 0),
        unit("b", &[], "run", 0, 0, 0, 0),
        unit("c", &[], "run", 0, 0, 0, 0),
    ];
    let found = outcomes(&[
        ("a", ProofOutcome::Failed),
        ("b", ProofOutcome::Failed),
        ("c", ProofOutcome::Failed),
    ]);
    let skip = BTreeSet::new();
    let full = failure_frontier(&units, &found, &caps(), &skip, 10).unwrap();
    let first = failure_frontier(&units, &found, &caps(), &skip, 1).unwrap();
    let text = first.continuation.as_ref().unwrap().text.clone();
    assert!(text.contains("\nb\n"), "{text}");
    assert!(text.contains("\nc\n"), "{text}");
    let second = resume_frontier(&units, &found, &caps(), &skip, &text, 1).unwrap();
    let third_text = second.continuation.as_ref().unwrap().text.clone();
    let third = resume_frontier(&units, &found, &caps(), &skip, &third_text, 10).unwrap();
    assert_eq!(third.failures, full.failures);
    assert!(third.continuation.is_none());
    assert_eq!(classes(&full).len(), 3);
}

#[test]
fn unresolved_outcome_keeps_a_lossless_continuation() {
    let units = vec![unit("a", &[], "run", 1, 1, 0, 0)];
    let frontier =
        failure_frontier(&units, &BTreeMap::new(), &caps(), &BTreeSet::new(), 10).unwrap();
    let continuation = frontier.continuation.expect("unresolved unit remains");
    assert!(continuation.remaining.contains("a"));
    assert!(continuation.text.contains("\na\n"));
    assert!(frontier.failures.is_empty());
    assert!(frontier.floor.contains("a"));
}

#[test]
fn tampered_continuation_is_refused() {
    let units = vec![unit("a", &[], "run", 0, 0, 0, 0)];
    let found = outcomes(&[("a", ProofOutcome::Failed)]);
    let frontier = failure_frontier(&units, &found, &caps(), &BTreeSet::new(), 0).unwrap();
    let text = frontier
        .continuation
        .unwrap()
        .text
        .replace("\na\n", "\nz\n");
    let error = resume_frontier(&units, &found, &caps(), &BTreeSet::new(), &text, 1).unwrap_err();
    assert_eq!(error, ScheduleError::Refused("continuation-mismatch"));
}

#[test]
fn dependency_legality_precedes_information_value() {
    let units = vec![
        unit("a", &[], "run", 10, 2, 0, 0),
        unit("c", &[], "run", 1, 0, 4, 1),
        unit("b", &[], "run", 9, 3, 0, 0),
        unit("d", &["b"], "run", 100, 1, 0, 0),
        unit("z", &[], "other", 1, 1, 0, 0),
    ];
    let order = legal_order(&units, &BTreeMap::new(), &caps(), &BTreeSet::new()).unwrap();
    assert_eq!(order, vec!["a".to_owned(), "c".to_owned(), "b".to_owned()]);
    let mut passed = outcomes(&[("b", ProofOutcome::Passed)]);
    let after = legal_order(&units, &passed, &caps(), &BTreeSet::new()).unwrap();
    assert_eq!(after[0], "d");
    passed.insert("a".to_owned(), ProofOutcome::Failed);
    passed.insert("d".to_owned(), ProofOutcome::Failed);
    passed.insert("z".to_owned(), ProofOutcome::Failed);
    let frontier = failure_frontier(&units, &passed, &caps(), &BTreeSet::new(), 1).unwrap();
    assert_eq!(
        classes(&frontier),
        BTreeSet::from([("z".to_owned(), CLASS_MISSING_CAPABILITY)])
    );
    let complete = failure_frontier(&units, &passed, &caps(), &BTreeSet::new(), 10).unwrap();
    assert!(classes(&complete).contains(&("z".to_owned(), CLASS_MISSING_CAPABILITY)));
    assert!(classes(&complete).contains(&("a".to_owned(), CLASS_PROOF_FAILED)));
    assert!(classes(&complete).contains(&("d".to_owned(), CLASS_PROOF_FAILED)));
    assert!(complete.floor.contains("d"));
    assert!(!complete.floor.contains("b"));
}

#[test]
fn zero_learned_cost_adds_no_ratio() {
    let units = vec![
        unit("a", &[], "run", 100, 0, 0, 0),
        unit("b", &[], "run", 1, 1, 0, 0),
    ];
    let order = legal_order(&units, &BTreeMap::new(), &caps(), &BTreeSet::new()).unwrap();
    assert_eq!(order, vec!["b".to_owned(), "a".to_owned()]);
}

#[test]
fn cycles_and_missing_dependencies_are_frontier_failures() {
    let units = vec![
        unit("a", &["b"], "run", 0, 0, 0, 0),
        unit("b", &["a"], "run", 0, 0, 0, 0),
        unit("m", &["ghost"], "run", 0, 0, 0, 0),
    ];
    let mut reversed = units.clone();
    reversed.reverse();
    let skip = BTreeSet::new();
    let empty = BTreeMap::new();
    let forward = failure_frontier(&units, &empty, &caps(), &skip, 10).unwrap();
    let backward = failure_frontier(&reversed, &empty, &caps(), &skip, 10).unwrap();
    assert_eq!(forward.failures, backward.failures);
    assert!(classes(&forward).contains(&("a".to_owned(), CLASS_CYCLE)));
    assert!(classes(&forward).contains(&("b".to_owned(), CLASS_CYCLE)));
    assert!(classes(&forward).contains(&("m".to_owned(), CLASS_MISSING_DEPENDENCY)));
    let detail = forward
        .failures
        .iter()
        .find(|failure| failure.unit == "m")
        .unwrap();
    assert_eq!(detail.detail, "ghost");
    let passed = outcomes(&[("a", ProofOutcome::Passed), ("b", ProofOutcome::Passed)]);
    let still = failure_frontier(&units, &passed, &caps(), &skip, 10).unwrap();
    assert!(classes(&still).contains(&("a".to_owned(), CLASS_CYCLE)));
    assert!(still.floor.contains("a"));
}

#[test]
fn a_failed_dependency_does_not_fail_the_dependent() {
    let units = vec![
        unit("slow", &[], "run", 1, 1, 0, 0),
        unit("hot", &["slow"], "run", 100, 1, 0, 0),
    ];
    let found = outcomes(&[
        ("slow", ProofOutcome::Failed),
        ("hot", ProofOutcome::Failed),
    ]);
    let frontier = failure_frontier(&units, &found, &caps(), &BTreeSet::new(), 10).unwrap();
    assert_eq!(
        classes(&frontier),
        BTreeSet::from([("slow".to_owned(), CLASS_PROOF_FAILED)])
    );
    assert!(frontier.floor.contains("hot"));
    assert!(frontier.continuation.is_none());
}

#[test]
fn duplicate_units_are_refused() {
    let units = vec![
        unit("a", &[], "run", 0, 0, 0, 0),
        unit("a", &[], "run", 0, 0, 0, 0),
    ];
    let error =
        failure_frontier(&units, &BTreeMap::new(), &caps(), &BTreeSet::new(), 1).unwrap_err();
    assert!(matches!(error, ScheduleError::DuplicateUnit(_)));
}

#[test]
fn legal_order_refuses_unknown_ids_and_skip_conflicts() {
    let units = vec![unit("a", &[], "run", 1, 1, 0, 0)];
    let unknown = legal_order(
        &units,
        &BTreeMap::new(),
        &caps(),
        &BTreeSet::from(["missing".to_owned()]),
    )
    .unwrap_err();
    assert!(matches!(unknown, ScheduleError::UnknownUnit(_)));
    let conflict = legal_order(
        &units,
        &outcomes(&[("a", ProofOutcome::Passed)]),
        &caps(),
        &BTreeSet::from(["a".to_owned()]),
    )
    .unwrap_err();
    assert_eq!(conflict, ScheduleError::Refused("skip-conflict"));
}

#[test]
fn empty_resource_amount_is_refused() {
    let mut specimen = unit("a", &[], "run", 0, 0, 0, 0);
    specimen.resources.push(ResourceAsk {
        kind: "cpu".to_owned(),
        amount: 0,
    });
    let error = legal_order(&[specimen], &BTreeMap::new(), &caps(), &BTreeSet::new()).unwrap_err();
    assert_eq!(error, ScheduleError::Refused("empty-resource"));
}
