// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use crate::schedule::CLASS_PROOF_FAILED;
use crate::{MAX_MUTATION_FILES, ProofOutcome};

use super::fixture::{accepted, complete, fact, malformed, opened, plan_for, proposal};
use super::{Descent, Fence, ProposalBody, Step, image_id, proof_frontier, propose};

#[test]
fn parallel_failures_stay_visible_without_reopening_claims() {
    let revision = opened();
    let claims = revision.claims.clone();
    let mut outcomes = BTreeMap::new();
    outcomes.insert("claim/alpha".to_owned(), ProofOutcome::Failed);
    outcomes.insert("claim/beta".to_owned(), ProofOutcome::Failed);
    outcomes.insert("claim/seal".to_owned(), ProofOutcome::Passed);
    let names = vec![
        "claim/alpha".to_owned(),
        "claim/beta".to_owned(),
        "claim/seal".to_owned(),
    ];
    let frontier = proof_frontier(&names, &outcomes).expect("frontier");
    let failed = frontier
        .failures
        .iter()
        .map(|failure| (failure.unit.as_str(), failure.class))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        failed,
        BTreeSet::from([
            ("claim/alpha", CLASS_PROOF_FAILED),
            ("claim/beta", CLASS_PROOF_FAILED),
        ])
    );
    assert!(frontier.continuation.is_none());
    assert_eq!(revision.claims, claims);
}

#[test]
fn oversize_is_refused_without_truncation() {
    let mut revision = opened();
    let before = accepted(&revision);
    let wide = vec![b'x'; 40];
    let step = propose(&mut revision, complete("fact/alpha", &wide, 39)).expect("propose");
    assert_eq!(
        step,
        Step::Refused {
            revision: revision.id.clone(),
            reason: "byte-ceiling",
        }
    );
    assert_eq!(wide.len(), 40);
    assert_eq!(accepted(&revision), before);
    assert_eq!(revision.images["fact/alpha"], b"alpha-0");
    let mut facts = BTreeMap::new();
    for index in 0..=MAX_MUTATION_FILES {
        facts.insert(format!("extra/{index}"), vec![b'x']);
    }
    let step = propose(
        &mut revision,
        proposal(
            "author",
            "checker",
            Fence::Commit,
            ProposalBody::Complete {
                id: "proposal/wide".to_owned(),
                facts,
                limit: usize::MAX,
            },
        ),
    )
    .expect("files");
    assert_eq!(
        step,
        Step::Refused {
            revision: revision.id.clone(),
            reason: "file-ceiling",
        }
    );
    assert_eq!(revision.images["fact/alpha"], b"alpha-0");
}

#[test]
fn incomplete_keeps_needs_and_does_not_commit() {
    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images.clone();
    let step = propose(
        &mut revision,
        proposal(
            "author",
            "author",
            Fence::Commit,
            ProposalBody::Incomplete {
                id: "proposal/need".to_owned(),
                remaining: vec!["fact/gamma".to_owned()],
            },
        ),
    )
    .expect("propose");
    assert_eq!(
        step,
        Step::Incomplete {
            revision: id.clone(),
            remaining: vec!["fact/gamma".to_owned()],
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.images, images);
    assert!(revision.claims.values().all(|claim| claim.closed));
}

#[test]
fn malformed_stays_honest_and_bounds_its_retry() {
    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images.clone();
    let claims = revision.claims.clone();
    let first = propose(&mut revision, malformed("shape", 4, None)).expect("first");
    assert_eq!(
        first,
        Step::Malformed {
            revision: id.clone(),
            reason: "shape".to_owned(),
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.images, images);
    assert_eq!(revision.claims, claims);
    let missing = propose(&mut revision, malformed("shape", 4, None)).expect("missing");
    assert_eq!(
        missing,
        Step::Refused {
            revision: id.clone(),
            reason: "descent",
        }
    );
    let retry = propose(
        &mut revision,
        malformed(
            "shape",
            2,
            Some(Descent {
                before: 4,
                after: 2,
            }),
        ),
    )
    .expect("retry");
    assert_eq!(
        retry,
        Step::Malformed {
            revision: id.clone(),
            reason: "shape".to_owned(),
        }
    );
    assert_eq!(revision.images, images);
    let third = propose(
        &mut revision,
        malformed(
            "shape",
            1,
            Some(Descent {
                before: 2,
                after: 1,
            }),
        ),
    )
    .expect("third");
    assert_eq!(
        third,
        Step::Refused {
            revision: id.clone(),
            reason: "retry-bound",
        }
    );
    assert_eq!(revision.claims, claims);
    let bare = propose(&mut revision, malformed("shape", 1, None)).expect("bare");
    assert_eq!(
        bare,
        Step::Refused {
            revision: id.clone(),
            reason: "retry-bound",
        }
    );
    assert_eq!(revision.images, images);
}

#[test]
fn identical_bytes_do_not_reopen_claims() {
    let mut revision = opened();
    let id = revision.id.clone();
    let image = image_id(&revision.images).expect("image");
    let claims = revision.claims.clone();
    let step = propose(&mut revision, complete("fact/alpha", b"alpha-0", 100)).expect("propose");
    assert_eq!(
        step,
        Step::Oscillation {
            revision: id.clone(),
            repeated: image,
            delta: BTreeSet::new(),
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.claims, claims);
    for fence in [Fence::BeforePerform, Fence::BeforeCommit] {
        let mut stopped = opened();
        let stopped_id = stopped.id.clone();
        let step = propose(
            &mut stopped,
            super::fixture::complete_fence("fact/alpha", b"alpha-0", fence),
        )
        .expect("stop");
        assert_eq!(
            step,
            Step::Oscillation {
                revision: stopped_id.clone(),
                repeated: image_id(&stopped.images).expect("image"),
                delta: BTreeSet::new(),
            }
        );
        assert_eq!(stopped.id, stopped_id);
        assert!(!stopped.uncertain);
    }
}

#[test]
fn unchanged_facts_are_left_out_of_the_sealed_plan() {
    let mut revision = opened();
    let plan = plan_for(b"alpha-0", b"alpha-1");
    let mut facts = fact("fact/alpha", b"alpha-1");
    facts.insert("fact/beta".to_owned(), b"beta-0".to_vec());
    let step = propose(
        &mut revision,
        proposal(
            "author",
            "checker",
            Fence::Commit,
            ProposalBody::Complete {
                id: "proposal/mixed".to_owned(),
                facts,
                limit: 100,
            },
        ),
    )
    .expect("propose");
    match step {
        Step::Invalidated {
            plan: plan_id,
            affected,
            closed,
            ..
        } => {
            assert_eq!(plan_id, plan.id);
            assert_eq!(affected, BTreeSet::from(["claim/alpha".to_owned()]));
            assert!(closed.contains("claim/beta"));
            assert!(revision.claims["claim/beta"].closed);
        }
        other => panic!("expected invalidation, got {other:?}"),
    }
}

#[test]
fn oscillation_witnesses_the_repeated_image() {
    let mut revision = opened();
    let original = image_id(&revision.images).expect("image");
    let first = propose(&mut revision, complete("fact/alpha", b"alpha-1", 100)).expect("first");
    assert!(matches!(first, Step::Invalidated { .. }));
    let accepted_id = revision.id.clone();
    assert_eq!(revision.images["fact/alpha"], b"alpha-1");
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(revision.claims["claim/beta"].closed);
    let back = propose(&mut revision, complete("fact/alpha", b"alpha-0", 100)).expect("back");
    assert_eq!(
        back,
        Step::Oscillation {
            revision: accepted_id.clone(),
            repeated: original,
            delta: BTreeSet::from(["fact/alpha".to_owned()]),
        }
    );
    assert_eq!(revision.id, accepted_id);
    assert_eq!(revision.images["fact/alpha"], b"alpha-1");
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(revision.claims["claim/beta"].closed);
    assert!(revision.claims["claim/seal"].closed);
}

#[test]
fn two_equal_proposals_share_the_plan_and_revision() {
    let mut first = opened();
    let mut second = opened();
    let plan = plan_for(b"alpha-0", b"alpha-1");
    let left = propose(&mut first, complete("fact/alpha", b"alpha-1", 100)).expect("left");
    let right = propose(&mut second, complete("fact/alpha", b"alpha-1", 100)).expect("right");
    match (left, right) {
        (
            Step::Invalidated {
                revision: left_id,
                plan: left_plan,
                ..
            },
            Step::Invalidated {
                revision: right_id,
                plan: right_plan,
                ..
            },
        ) => {
            assert_eq!(left_id, right_id);
            assert_eq!(left_plan, right_plan);
            assert_eq!(left_plan, plan.id);
            assert_eq!(first.id, second.id);
        }
        other => panic!("expected two commits, got {other:?}"),
    }
}

#[test]
fn unknown_fact_is_refused() {
    let mut revision = opened();
    let id = revision.id.clone();
    let step = propose(
        &mut revision,
        proposal(
            "author",
            "checker",
            Fence::Commit,
            ProposalBody::Complete {
                id: "proposal/missing".to_owned(),
                facts: fact("fact/missing", b"nope"),
                limit: 100,
            },
        ),
    )
    .expect("propose");
    assert_eq!(
        step,
        Step::Refused {
            revision: id.clone(),
            reason: "unknown-fact",
        }
    );
    assert_eq!(revision.id, id);
}
