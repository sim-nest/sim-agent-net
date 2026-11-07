// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: sealed fixed point.

use std::collections::{BTreeMap, BTreeSet};

use super::fixture::{calls, paths, seal, world, world_alpha_unclaimed};
use super::{FactGenerator, FixedStep, retire_duplicate, seal_fixed_point};
use crate::{
    Fence, OpenClaim, Proposal, ProposalBody, RevisionOpen, SOURCE_SEAL, Step, image_id,
    open_revision, propose, seal_id,
};

#[test]
fn regeneration_matches_and_the_second_pass_does_no_work() {
    let world = world();
    let accepted = world.revision.clone();
    let sealed = seal(&world, &BTreeSet::new(), None).expect("seal");
    let FixedStep::Sealed { seal: first, work } = sealed else {
        panic!("expected a seal, got {sealed:?}");
    };
    assert_eq!(work, 2);
    assert_eq!(calls(&world), 2);
    assert_eq!(world.revision, accepted);
    let seen = world.alpha.seen.borrow().clone().expect("source");
    assert_eq!(
        seen.keys().cloned().collect::<BTreeSet<_>>(),
        paths(&["fact/src", SOURCE_SEAL])
    );
    assert_eq!(seen.get("fact/src"), accepted.images().get("fact/src"));
    assert_eq!(seen.get(SOURCE_SEAL), accepted.images().get(SOURCE_SEAL));
    assert!(!seen.contains_key("fact/alpha"));
    let beta_seen = world.beta.seen.borrow().clone().expect("beta source");
    assert_eq!(
        beta_seen.keys().cloned().collect::<BTreeSet<_>>(),
        paths(&["fact/src"])
    );
    assert!(!beta_seen.contains_key("fact/beta"));
    assert_eq!(world.alpha.owner, *world.facets[1].owner());

    world.alpha.set("fact/alpha", b"drift");
    let again = seal(&world, &BTreeSet::new(), Some(&first)).expect("second");
    assert_eq!(
        again,
        FixedStep::ZeroWork {
            seal: first.clone()
        }
    );
    assert_eq!(calls(&world), 2);
    assert_eq!(world.revision, accepted);

    let mut forged = first.clone();
    forged.id = seal_id(b"forged").expect("forged");
    world.alpha.set("fact/alpha", b"alpha");
    let resealed = seal(&world, &BTreeSet::new(), Some(&forged)).expect("reseal");
    let FixedStep::Sealed { seal: fresh, work } = resealed else {
        panic!("expected a reseal, got {resealed:?}");
    };
    assert_eq!(work, 2);
    assert_eq!(fresh, first);
    assert_eq!(calls(&world), 4);
}

#[test]
fn deletion_invalidates_exactly_that_generator_closure() {
    let world = world();
    let sealed = seal(&world, &BTreeSet::new(), None).expect("seal");
    let FixedStep::Sealed { seal: prior, .. } = sealed else {
        panic!("expected a seal");
    };
    let accepted = world.revision.clone();
    let step = seal(&world, &paths(&["fact/alpha"]), Some(&prior)).expect("delete");
    let FixedStep::Invalidated { closure, retained } = step else {
        panic!("expected invalidation, got {step:?}");
    };
    assert_eq!(
        closure,
        paths(&["fact/alpha", "fact/extra", "claim/alpha", "claim/extra"])
    );
    assert_eq!(retained, paths(&["fact/beta"]));
    assert!(!closure.contains("claim/beta"));
    assert!(!closure.contains("claim/src"));
    assert!(!closure.contains("fact/src"));
    assert_eq!(world.revision, accepted);
    assert_eq!(calls(&world), 2);
}

#[test]
fn a_claim_id_is_not_a_fact_path_for_later_claims() {
    let base = world();
    let seal_bytes = base
        .revision
        .images()
        .get(SOURCE_SEAL)
        .expect("seal")
        .clone();
    let revision = open_revision(RevisionOpen {
        seal_id: seal_id(&seal_bytes).expect("id"),
        seal: seal_bytes,
        images: base.revision.images().clone(),
        claims: vec![
            OpenClaim {
                id: "fact/beta".to_owned(),
                inputs: paths(&["fact/alpha"]),
            },
            OpenClaim {
                id: "claim/later".to_owned(),
                inputs: paths(&["fact/beta"]),
            },
            OpenClaim {
                id: "claim/extra".to_owned(),
                inputs: paths(&["fact/extra"]),
            },
        ],
    })
    .expect("revision");
    let accepted = revision.clone();
    let generators: [&dyn FactGenerator; 2] = [&base.alpha, &base.beta];
    let step = seal_fixed_point(
        &revision,
        &base.facets,
        &generators,
        &BTreeMap::new(),
        &paths(&["fact/alpha"]),
        None,
    )
    .expect("delete");
    let FixedStep::Invalidated { closure, retained } = step else {
        panic!("expected invalidation, got {step:?}");
    };
    assert_eq!(
        closure,
        paths(&["claim/extra", "fact/alpha", "fact/beta", "fact/extra"])
    );
    assert_eq!(retained, paths(&["fact/beta"]));
    assert!(!closure.contains("claim/later"));
    assert_eq!(revision, accepted);
    assert_eq!(calls(&base), 0);
}

#[test]
fn a_repeated_image_blocks_without_replacing_the_revision() {
    let mut world = world_alpha_unclaimed();
    let original = image_id(world.revision.images()).expect("image");
    let step = propose(
        &mut world.revision,
        Proposal {
            author: "author".to_owned(),
            checker: "checker".to_owned(),
            fence: Fence::Commit,
            body: ProposalBody::Complete {
                id: "proposal/1".to_owned(),
                facts: one("fact/alpha", b"alpha-b"),
                limit: 100,
            },
        },
    )
    .expect("propose");
    let Step::Invalidated { affected, .. } = step else {
        panic!("expected a commit, got {step:?}");
    };
    assert!(affected.is_empty());
    assert!(world.revision.claims().values().all(|claim| claim.closed));
    let accepted = world.revision.clone();
    assert!(world.revision.has_seen(&original));
    let judged = seal(&world, &BTreeSet::new(), None).expect("seal");
    assert_eq!(
        judged,
        FixedStep::Blocked {
            reason: "oscillation",
            closure: paths(&["fact/alpha"]),
            repeated: Some(original),
        }
    );
    assert_eq!(world.revision, accepted);
    assert_eq!(
        world.revision.images().get("fact/alpha").map(Vec::as_slice),
        Some(b"alpha-b".as_slice())
    );
}

#[test]
fn a_new_or_empty_postimage_is_a_gap_and_is_not_written() {
    let world = world();
    let accepted = world.revision.clone();
    world.alpha.set("fact/alpha", b"alpha-c");
    let mismatched = seal(&world, &BTreeSet::new(), None).expect("gap");
    assert_eq!(mismatched, refusal_gap(&["fact/alpha"]));
    assert_eq!(world.revision, accepted);

    world.alpha.set("fact/alpha", b"");
    let empty = seal(&world, &BTreeSet::new(), None).expect("empty");
    assert_eq!(empty, refusal_gap(&["fact/alpha"]));
    assert_eq!(
        world.revision.images().get("fact/alpha").map(Vec::as_slice),
        Some(b"alpha".as_slice())
    );
    assert_eq!(world.revision, accepted);
}

#[test]
fn retire_duplicate_requires_a_zero_delta_before_the_retained_generator_seals() {
    let world = world();
    let accepted = world.revision.clone();
    let duplicate = world.alpha.twin();
    duplicate.set("fact/alpha", b"other");
    let retired = retire_duplicate(&world.revision, &world.alpha, &duplicate).expect("retire");
    assert_eq!(retired, FixedStep::Retired { work: 1 });
    assert_eq!(world.alpha.calls.get(), 1);
    assert_eq!(duplicate.calls.get(), 0);
    assert_eq!(world.revision, accepted);

    let sealed = seal(&world, &BTreeSet::new(), None).expect("seal");
    assert!(matches!(sealed, FixedStep::Sealed { work: 2, .. }));
    assert_eq!(duplicate.calls.get(), 0);

    world.alpha.set("fact/alpha", b"other");
    let drifted = retire_duplicate(&world.revision, &world.alpha, &duplicate).expect("drift");
    assert_eq!(
        drifted,
        super::fixture::refusal("not-zero", &["fact/alpha"])
    );
    assert_eq!(world.revision, accepted);

    let narrower = super::fixture::Gen::new(
        &world.alpha.owner,
        &["fact/src"],
        &[("fact/alpha", b"alpha")],
    );
    let split = retire_duplicate(&world.revision, &world.alpha, &narrower).expect("split");
    assert_eq!(
        split,
        super::fixture::refusal("different-closure", &["fact/extra"])
    );
    assert_eq!(narrower.calls.get(), 0);
    assert_eq!(world.revision, accepted);
}

fn one(path: &str, bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut facts = std::collections::BTreeMap::new();
    facts.insert(path.to_owned(), bytes.to_vec());
    facts
}

fn refusal_gap(closure: &[&str]) -> FixedStep {
    super::fixture::refusal("gap", closure)
}
