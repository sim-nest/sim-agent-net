// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use super::fixture::{claim, complete, complete_fence, opened, opened_without, plan_for, proposal};
use super::{
    ConvergeError, Fence, OpenRequest, ProposalBody, SOURCE_SEAL, Step, image_id, open, propose,
    seal_id,
};

#[test]
fn open_refuses_a_bad_seal() {
    let seal = b"seal-v1".to_vec();
    let mut images = BTreeMap::new();
    images.insert("fact/alpha".to_owned(), b"alpha-0".to_vec());
    let wrong = open(OpenRequest {
        seal: seal.clone(),
        seal_id: seal_id(b"other").expect("other"),
        images: images.clone(),
        claims: vec![claim("claim/alpha", &["fact/alpha"])],
    });
    assert_eq!(wrong.err(), Some(ConvergeError::SealMismatch));
    let empty = open(OpenRequest {
        seal: Vec::new(),
        seal_id: seal_id(b"seal-v1").expect("seal"),
        images,
        claims: vec![claim("claim/alpha", &["fact/alpha"])],
    });
    assert_eq!(empty.err(), Some(ConvergeError::EmptySeal));
    let missing = opened_without("claim/missing", &["fact/missing"]);
    assert_eq!(missing.err(), Some(ConvergeError::UnknownInput));
}

#[test]
fn open_refuses_a_path_the_mutation_layer_rejects() {
    let seal = b"seal-v1".to_vec();
    let seal_id = seal_id(&seal).expect("seal");
    let mut escape = BTreeMap::new();
    escape.insert("../escape".to_owned(), b"x".to_vec());
    let escaped = open(OpenRequest {
        seal: seal.clone(),
        seal_id: seal_id.clone(),
        images: escape,
        claims: Vec::new(),
    });
    assert_eq!(escaped.err(), Some(ConvergeError::Mutation("invalid-path")));
    let mut nested = BTreeMap::new();
    nested.insert("fact".to_owned(), b"a".to_vec());
    nested.insert("fact/alpha".to_owned(), b"b".to_vec());
    let parent = open(OpenRequest {
        seal,
        seal_id,
        images: nested,
        claims: Vec::new(),
    });
    assert_eq!(parent.err(), Some(ConvergeError::Mutation("parent-child")));
}

#[test]
fn equal_opens_share_a_revision_id() {
    let first = opened();
    let second = opened();
    assert_eq!(first.id, second.id);
    assert_eq!(
        first.seal,
        seal_id(&first.images[SOURCE_SEAL]).expect("seal")
    );
    assert_eq!(
        image_id(&first.images).expect("image"),
        image_id(&second.images).expect("image")
    );
    assert!(
        first
            .claims
            .values()
            .all(|claim| claim.closed && claim.digest.is_some())
    );
}

#[test]
fn crash_before_perform_keeps_the_accepted_revision() {
    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images.clone();
    let claims = revision.claims.clone();
    let seen = revision.seen_images.clone();
    let step = propose(
        &mut revision,
        complete_fence("fact/alpha", b"alpha-1", Fence::BeforePerform),
    )
    .expect("propose");
    assert_eq!(
        step,
        Step::Held {
            revision: id.clone()
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.images, images);
    assert_eq!(revision.claims, claims);
    assert_eq!(revision.seen_images, seen);
    assert!(!revision.uncertain);
    let committed =
        propose(&mut revision, complete("fact/alpha", b"alpha-1", 100)).expect("commit");
    assert!(matches!(committed, Step::Invalidated { .. }));
    assert_ne!(revision.id, id);
    assert_eq!(revision.images["fact/alpha"], b"alpha-1");
}

#[test]
fn crash_before_commit_blocks_another_commit() {
    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images.clone();
    let claims = revision.claims.clone();
    let step = propose(
        &mut revision,
        complete_fence("fact/alpha", b"alpha-1", Fence::BeforeCommit),
    )
    .expect("propose");
    assert_eq!(
        step,
        Step::Held {
            revision: id.clone()
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.images, images);
    assert_eq!(revision.claims, claims);
    assert!(revision.uncertain);
    let retry = propose(&mut revision, complete("fact/alpha", b"alpha-1", 100)).expect("retry");
    assert_eq!(
        retry,
        Step::Refused {
            revision: id.clone(),
            reason: "uncertain",
        }
    );
    assert_eq!(revision.images, images);
    assert_eq!(revision.claims, claims);
}

#[test]
fn author_cannot_approve_their_own_proposal() {
    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images.clone();
    let step = propose(
        &mut revision,
        proposal(
            "ada",
            "ada",
            Fence::Commit,
            ProposalBody::Complete {
                id: "proposal/self".to_owned(),
                facts: super::fixture::fact("fact/alpha", b"alpha-1"),
                limit: 100,
            },
        ),
    )
    .expect("propose");
    assert_eq!(
        step,
        Step::Refused {
            revision: id.clone(),
            reason: "self-approval",
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.images, images);
    let empty = propose(
        &mut revision,
        proposal(
            "",
            "checker",
            Fence::Commit,
            ProposalBody::Complete {
                id: "proposal/empty".to_owned(),
                facts: super::fixture::fact("fact/alpha", b"alpha-1"),
                limit: 100,
            },
        ),
    )
    .expect("empty");
    assert_eq!(
        empty,
        Step::Refused {
            revision: id.clone(),
            reason: "empty-identity",
        }
    );
}

#[test]
fn changed_fact_invalidates_only_its_claims() {
    let mut revision = opened();
    let beta = revision.claims["claim/beta"].digest.clone();
    let seal_claim = revision.claims["claim/seal"].digest.clone();
    let plan = plan_for(b"alpha-0", b"alpha-1");
    let step = propose(&mut revision, complete("fact/alpha", b"alpha-1", 100)).expect("propose");
    match step {
        Step::Invalidated {
            revision: next,
            plan: plan_id,
            affected,
            closed,
        } => {
            assert_eq!(revision.id, next);
            assert_eq!(plan_id, plan.id);
            assert_eq!(affected, BTreeSet::from(["claim/alpha".to_owned()]));
            assert!(closed.contains("claim/beta"));
            assert!(closed.contains("claim/seal"));
            assert!(!closed.contains("claim/alpha"));
        }
        other => panic!("expected invalidation, got {other:?}"),
    }
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(revision.claims["claim/alpha"].digest.is_none());
    assert!(revision.claims["claim/beta"].closed);
    assert_eq!(revision.claims["claim/beta"].digest, beta);
    assert!(revision.claims["claim/seal"].closed);
    assert_eq!(revision.claims["claim/seal"].digest, seal_claim);
    assert_eq!(revision.images["fact/alpha"], b"alpha-1");
    assert_eq!(revision.images["fact/beta"], b"beta-0");
}

#[test]
fn seal_change_invalidates_only_the_seal_claim() {
    let mut revision = opened();
    let alpha = revision.claims["claim/alpha"].digest.clone();
    let step = propose(&mut revision, complete(SOURCE_SEAL, b"seal-v2", 100)).expect("propose");
    let Step::Invalidated {
        affected, closed, ..
    } = step
    else {
        panic!("expected invalidation");
    };
    assert_eq!(affected, BTreeSet::from(["claim/seal".to_owned()]));
    assert!(closed.contains("claim/alpha"));
    assert!(closed.contains("claim/beta"));
    assert_eq!(revision.claims["claim/alpha"].digest, alpha);
    assert_eq!(revision.seal, seal_id(b"seal-v2").expect("seal"));
    assert_eq!(revision.images[SOURCE_SEAL], b"seal-v2");
}

#[test]
fn empty_source_seal_is_refused_without_a_commit() {
    let mut revision = opened();
    let id = revision.id.clone();
    let seal = revision.seal.clone();
    let images = revision.images.clone();
    let held = propose(
        &mut revision,
        complete_fence(SOURCE_SEAL, b"", Fence::BeforeCommit),
    )
    .expect("held");
    assert_eq!(
        held,
        Step::Refused {
            revision: id.clone(),
            reason: "empty-seal",
        }
    );
    assert!(!revision.uncertain);
    let step = propose(&mut revision, complete(SOURCE_SEAL, b"", 100)).expect("propose");
    assert_eq!(
        step,
        Step::Refused {
            revision: id.clone(),
            reason: "empty-seal",
        }
    );
    assert_eq!(revision.id, id);
    assert_eq!(revision.seal, seal);
    assert_eq!(revision.images, images);
    assert!(revision.claims.values().all(|claim| claim.closed));
}

#[test]
fn claim_inputs_stay_in_the_revision_id_after_they_open() {
    let seal = b"seal-v1".to_vec();
    let seal_content = seal_id(&seal).expect("seal");
    let mut images = BTreeMap::new();
    images.insert("fact/alpha".to_owned(), b"alpha-0".to_vec());
    images.insert("fact/beta".to_owned(), b"beta-0".to_vec());
    let mut narrow = open(OpenRequest {
        seal: seal.clone(),
        seal_id: seal_content.clone(),
        images: images.clone(),
        claims: vec![claim("claim/alpha", &["fact/alpha"])],
    })
    .expect("narrow");
    let mut wide = open(OpenRequest {
        seal,
        seal_id: seal_content,
        images,
        claims: vec![claim("claim/alpha", &["fact/alpha", "fact/beta"])],
    })
    .expect("wide");
    assert_ne!(narrow.id, wide.id);
    let left = propose(&mut narrow, complete("fact/alpha", b"alpha-1", 100)).expect("left");
    let right = propose(&mut wide, complete("fact/alpha", b"alpha-1", 100)).expect("right");
    assert!(matches!(left, Step::Invalidated { .. }));
    assert!(matches!(right, Step::Invalidated { .. }));
    assert!(narrow.claims["claim/alpha"].digest.is_none());
    assert!(wide.claims["claim/alpha"].digest.is_none());
    assert_eq!(narrow.images["fact/alpha"], wide.images["fact/alpha"]);
    assert_eq!(narrow.seal, wide.seal);
    assert_ne!(narrow.id, wide.id);
}
