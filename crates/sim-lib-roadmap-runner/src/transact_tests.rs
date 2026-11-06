// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sim_artifact_facet::MergePolicy;

use super::{ArtifactModel, CleanKind, GitModel, RepoOutcome, transact_facets};
use crate::transact::fixture::{
    advance, applied_tip, apply, edit, exact, lines, linewise, owner, public_disclosure, refusal,
    refusal_at, request, seeded, text,
};

#[test]
fn a_direct_apply_commits_only_that_repository() {
    let actor = owner("owner/src");
    let (mut store, alpha) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let beta = store
        .insert_repository(
            "beta",
            "main",
            crate::transact::fixture::tree(&[("src/a.txt", "old\n")]),
        )
        .expect("beta");
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &alpha,
            &alpha.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("batch");
    assert!(batch.settled());
    let RepoOutcome::Applied { previous, tip, .. } = &batch.outcomes()[0] else {
        panic!("expected a commit: {batch:?}");
    };
    assert_eq!(previous, &alpha);
    assert_ne!(tip.head, alpha.head);
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("new\n")
    );
    assert_eq!(
        store.image_at("alpha", &alpha.head, "src/a.txt"),
        text("old\n")
    );
    assert_eq!(store.tip("beta"), Some(beta.clone()));
    assert_eq!(
        store.image_at("beta", &beta.head, "src/a.txt"),
        text("old\n")
    );
}

#[test]
fn two_regions_on_one_file_keep_both_edits() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "a\nb\nc\n")]);
    let policy = linewise();
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![
                edit(
                    lines("src/a.txt", &actor, 0, 1, policy.clone()),
                    text("a\nb\nc\n"),
                    text("A\nb\nc\n"),
                ),
                edit(
                    lines("src/a.txt", &actor, 2, 3, policy),
                    text("a\nb\nc\n"),
                    text("a\nb\nC\n"),
                ),
            ],
        )],
    )
    .expect("batch");
    let next = applied_tip(&batch);
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("A\nb\nC\n")
    );
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("a\nb\nc\n")
    );
}

#[test]
fn a_disjoint_foreign_edit_merges_and_an_overlap_does_not() {
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
    let merged = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, linewise()),
                text("a\nb\nc\n"),
                text("a\nb\nC\n"),
            )],
        )],
    )
    .expect("merge");
    let next = applied_tip(&merged);
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("A\nb\nC\n")
    );

    let (mut other, base) = seeded("beta", &[("src/a.txt", "a\nb\nc\n")]);
    let tip = advance(
        &mut other,
        "beta",
        &base,
        &actor,
        "src/a.txt",
        text("A\nb\nc\n"),
    );
    let refused = transact_facets(
        &mut other,
        &[request(
            "beta",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, linewise()),
                text("a\nb\nc\n"),
                text("X\nb\nc\n"),
            )],
        )],
    )
    .expect("overlap");
    assert_eq!(refusal(&refused), "overlap");
    assert_eq!(other.tip("beta"), Some(tip.clone()));
    assert_eq!(
        other.image_at("beta", &tip.head, "src/a.txt"),
        text("A\nb\nc\n")
    );
}

#[test]
fn overlap_is_symmetric_when_the_two_sides_swap() {
    let actor = owner("owner/src");
    for (observed, intended) in [("A\nb\n", "B\nb\n"), ("B\nb\n", "A\nb\n")] {
        let (mut store, base) = seeded("alpha", &[("src/a.txt", "a\nb\n")]);
        let tip = advance(
            &mut store,
            "alpha",
            &base,
            &actor,
            "src/a.txt",
            text(observed),
        );
        let batch = transact_facets(
            &mut store,
            &[request(
                "alpha",
                &actor,
                &tip,
                &base.head,
                public_disclosure(),
                vec![edit(
                    exact("src/a.txt", &actor, linewise()),
                    text("a\nb\n"),
                    text(intended),
                )],
            )],
        )
        .expect("batch");
        assert_eq!(refusal(&batch), "overlap");
        assert_eq!(
            store.image_at("alpha", &tip.head, "src/a.txt"),
            text(observed)
        );
    }
}

#[test]
fn already_true_and_unchanged_do_not_move_the_branch() {
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
    let already = transact_facets(
        &mut store,
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
    .expect("already");
    assert!(already.settled());
    assert!(matches!(
        &already.outcomes()[0],
        RepoOutcome::Clean {
            kind: CleanKind::AlreadyTrue,
            tip: current,
            ..
        } if current == &tip
    ));
    assert_eq!(store.tip("alpha"), Some(tip.clone()));

    let unchanged = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("old\n"),
            )],
        )],
    )
    .expect("unchanged");
    assert!(matches!(
        &unchanged.outcomes()[0],
        RepoOutcome::Clean {
            kind: CleanKind::Unchanged,
            ..
        }
    ));
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("new\n")
    );
}

#[test]
fn a_merged_sibling_of_already_true_stays_unchanged() {
    let actor = owner("owner/src");
    let (mut store, base) = seeded("alpha", &[("src/a.txt", "a\nb\nc\n")]);
    let tip = advance(
        &mut store,
        "alpha",
        &base,
        &actor,
        "src/a.txt",
        text("A\nb\nC\n"),
    );
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![
                edit(
                    exact("src/a.txt", &actor, linewise()),
                    text("a\nb\nc\n"),
                    text("A\nb\nC\n"),
                ),
                edit(
                    exact("src/a.txt", &actor, linewise()),
                    text("a\nb\nc\n"),
                    text("a\nb\nC\n"),
                ),
            ],
        )],
    )
    .expect("batch");
    assert!(batch.settled());
    assert!(matches!(
        &batch.outcomes()[0],
        RepoOutcome::Clean {
            kind: CleanKind::Unchanged,
            tip: current,
            ..
        } if current == &tip
    ));
    assert_eq!(store.tip("alpha"), Some(tip.clone()));
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("A\nb\nC\n")
    );
}

#[test]
fn a_second_pass_on_the_new_head_performs_no_commit() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let next = apply(&mut store, &tip, &actor, "src/a.txt", "new\n");
    let again = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &next,
            &next.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("new\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("again");
    assert!(matches!(
        &again.outcomes()[0],
        RepoOutcome::Clean {
            kind: CleanKind::Unchanged,
            tip: current,
            ..
        } if current == &next
    ));
}

#[test]
fn a_later_request_keeps_the_first_commit_when_its_head_is_stale() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let next = apply(&mut store, &tip, &actor, "src/a.txt", "new\n");
    let mut stale = request(
        "alpha",
        &actor,
        &tip,
        &tip.head,
        public_disclosure(),
        vec![edit(
            exact("src/a.txt", &actor, MergePolicy::Exact),
            text("old\n"),
            text("other\n"),
        )],
    );
    stale.expected_head = tip.head.clone();
    let batch = transact_facets(&mut store, &[stale]).expect("stale");
    assert_eq!(refusal(&batch), "wrong-head");
    assert_eq!(store.tip("alpha"), Some(next.clone()));
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("new\n")
    );
}

#[test]
fn one_repository_refusal_does_not_rewind_another() {
    let actor = owner("owner/src");
    let (mut store, alpha) = seeded("alpha", &[("src/a.txt", "old\n")]);
    store
        .insert_repository(
            "beta",
            "main",
            crate::transact::fixture::tree(&[("src/b.txt", "a\nb\n")]),
        )
        .expect("beta");
    let beta = store.tip("beta").expect("beta tip");
    let beta_tip = advance(
        &mut store,
        "beta",
        &beta,
        &actor,
        "src/b.txt",
        text("A\nb\n"),
    );
    let batch = transact_facets(
        &mut store,
        &[
            request(
                "alpha",
                &actor,
                &alpha,
                &alpha.head,
                public_disclosure(),
                vec![edit(
                    exact("src/a.txt", &actor, MergePolicy::Exact),
                    text("old\n"),
                    text("new\n"),
                )],
            ),
            request(
                "beta",
                &actor,
                &beta_tip,
                &beta.head,
                public_disclosure(),
                vec![edit(
                    exact("src/b.txt", &actor, linewise()),
                    text("a\nb\n"),
                    text("B\nb\n"),
                )],
            ),
        ],
    )
    .expect("batch");
    assert!(!batch.settled());
    assert!(matches!(batch.outcomes()[0], RepoOutcome::Applied { .. }));
    assert_eq!(refusal_at(&batch, 1), "overlap");
    let applied = applied_tip(&batch);
    assert_eq!(
        store.image_at("alpha", &applied.head, "src/a.txt"),
        text("new\n")
    );
    assert_eq!(
        store.image_at("beta", &beta_tip.head, "src/b.txt"),
        text("A\nb\n")
    );
}

#[test]
fn two_requests_for_one_repository_do_not_collapse_into_one_commit() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let first = request(
        "alpha",
        &actor,
        &tip,
        &tip.head,
        public_disclosure(),
        vec![edit(
            exact("src/a.txt", &actor, MergePolicy::Exact),
            text("old\n"),
            text("one\n"),
        )],
    );
    let second = request(
        "alpha",
        &actor,
        &tip,
        &tip.head,
        public_disclosure(),
        vec![edit(
            exact("src/a.txt", &actor, MergePolicy::Exact),
            text("old\n"),
            text("two\n"),
        )],
    );
    let batch = transact_facets(&mut store, &[first, second]).expect("batch");
    assert!(!batch.settled());
    assert_eq!(refusal_at(&batch, 1), "wrong-head");
    let next = applied_tip(&batch);
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("one\n")
    );
}
