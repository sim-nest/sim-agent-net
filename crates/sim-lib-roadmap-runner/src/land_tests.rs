// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: a landing answer follows the branch and refuses a stale head.

use sim_artifact_facet::MergePolicy;

use super::{
    LandCache, LandPlan, LandStatus, LandWhy, land_apply, land_plan, land_recover, land_status,
    land_why,
};
use crate::transact::fixture::{
    advance, edit, exact, generated, linewise, owner, public_disclosure, request, seeded, text,
};
use crate::transact::{ArtifactModel, CleanKind, GitModel, RepoOutcome, transact_facets};

#[test]
fn a_foreign_edit_survives_plan_and_apply() {
    let actor = owner("owner/src");
    let (mut store, base) = seeded("alpha", &[("src/a.txt", "a\nb\nc\n")]);
    let tip = advance(
        &mut store,
        "alpha",
        &base,
        &actor,
        "src/a.txt",
        text("A\nb\nc\n"),
    );
    let mut cache = LandCache::new();
    let edit = edit(
        exact("src/a.txt", &actor, linewise()),
        text("a\nb\nc\n"),
        text("a\nb\nC\n"),
    );
    let planned = land_plan(
        &store,
        &mut cache,
        &request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit.clone()],
        ),
    )
    .expect("plan");
    let LandPlan::WouldCommit {
        files, tip: seen, ..
    } = &planned
    else {
        panic!("expected a commit plan: {planned:?}");
    };
    assert_eq!(seen, &tip);
    assert_eq!(files.get("src/a.txt"), Some(&text("A\nb\nC\n")));
    assert_eq!(store.tip("alpha"), Some(tip.clone()));
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("A\nb\nc\n")
    );

    let batch = land_apply(
        &mut store,
        &mut cache,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit],
        )],
    )
    .expect("apply");
    let RepoOutcome::Applied { tip: next, .. } = &batch.outcomes()[0] else {
        panic!("expected a commit: {batch:?}");
    };
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("A\nb\nC\n")
    );
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("A\nb\nc\n")
    );
}

#[test]
fn an_already_true_replay_does_not_move_the_branch() {
    let actor = owner("owner/src");
    let (mut store, base) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let tip = advance(
        &mut store,
        "alpha",
        &base,
        &actor,
        "src/a.txt",
        text("new\n"),
    );
    let mut cache = LandCache::new();
    let again = land_apply(
        &mut store,
        &mut cache,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("replay");
    assert!(matches!(
        &again.outcomes()[0],
        RepoOutcome::Clean {
            kind: CleanKind::AlreadyTrue,
            tip: current,
            ..
        } if current == &tip
    ));
    assert_eq!(store.tip("alpha"), Some(tip));
}

#[test]
fn recover_restores_the_exact_commit() {
    let actor = owner("owner/src");
    let (mut store, base) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let mut cache = LandCache::new();
    let batch = land_apply(
        &mut store,
        &mut cache,
        &[request(
            "alpha",
            &actor,
            &base,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("apply");
    let RepoOutcome::Applied {
        tip: next,
        previous,
        ..
    } = &batch.outcomes()[0]
    else {
        panic!("expected a commit: {batch:?}");
    };
    let restored =
        land_recover(&mut store, &mut cache, "alpha", next, &previous.head).expect("recover");
    let crate::transact::Rollback::Restored { tip } = &restored else {
        panic!("expected a restored pointer: {restored:?}");
    };
    assert_eq!(tip, previous);
    assert_eq!(
        store.image_at("alpha", &previous.head, "src/a.txt"),
        text("old\n")
    );
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("new\n")
    );
    assert!(matches!(
        land_why(&store, &cache, "alpha"),
        LandWhy::Current { text, head, .. } if text == "restored" && head == previous.head
    ));
}

#[test]
fn a_generated_region_refuses_another_actor() {
    let generator = owner("owner/src");
    let other = owner("owner/other");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let mut cache = LandCache::new();
    let planned = land_plan(
        &store,
        &mut cache,
        &request(
            "alpha",
            &other,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                generated("src/a.txt", &generator, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        ),
    )
    .expect("plan");
    assert!(matches!(
        &planned,
        LandPlan::Refused { reason, .. } if *reason == "generated-owner-bypass"
    ));
    assert_eq!(store.tip("alpha"), Some(tip.clone()));
    let applied = land_apply(
        &mut store,
        &mut cache,
        &[request(
            "alpha",
            &other,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                generated("src/a.txt", &generator, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("apply");
    assert!(matches!(
        &applied.outcomes()[0],
        RepoOutcome::Refused { reason, .. } if *reason == "generated-owner-bypass"
    ));
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
}

#[test]
fn a_cached_answer_stays_until_the_head_moves() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let mut cache = LandCache::new();
    assert!(matches!(
        land_why(&store, &cache, "alpha"),
        LandWhy::Unknown { .. }
    ));
    let planned = land_plan(
        &store,
        &mut cache,
        &request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        ),
    )
    .expect("plan");
    assert!(matches!(planned, LandPlan::WouldCommit { .. }));
    let first = land_why(&store, &cache, "alpha");
    let second = land_why(&store, &cache, "alpha");
    assert_eq!(first, second);
    assert!(matches!(
        &first,
        LandWhy::Current { text, head, .. } if text == "would-commit" && head == &tip.head
    ));
    assert_eq!(store.tip("alpha"), Some(tip.clone()));

    transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("direct commit");
    assert!(matches!(
        land_why(&store, &cache, "alpha"),
        LandWhy::Stale { cached_head, current_head, .. }
            if cached_head == tip.head && current_head != tip.head
    ));
}

#[test]
fn status_names_the_tip_and_a_missing_repository() {
    let (store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    assert!(matches!(
        land_status(&store, "alpha"),
        LandStatus::Present { tip: seen, dirty, .. } if seen == tip && dirty.is_empty()
    ));
    assert!(matches!(
        land_status(&store, "missing"),
        LandStatus::Missing { .. }
    ));
}
