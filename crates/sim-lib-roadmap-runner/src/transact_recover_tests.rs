// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: each model repository commits or refuses on its own.

use sim_artifact_facet::{MergePolicy, PortableImage};

use super::{ArtifactModel, GitModel, Rollback, rollback_model, transact_facets};
use crate::transact::fixture::{
    applied_tip, edit, exact, lines, linewise, owner, public_disclosure, refusal, request, seeded,
    text,
};

#[test]
fn a_region_escape_after_a_lawful_edit_writes_nothing() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded(
        "alpha",
        &[("src/a.txt", "old\n"), ("src/b.txt", "a\nb\nc\n")],
    );
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
                    exact("src/a.txt", &actor, MergePolicy::Exact),
                    text("old\n"),
                    text("new\n"),
                ),
                edit(
                    lines("src/b.txt", &actor, 2, 3, linewise()),
                    text("a\nb\nc\n"),
                    text("Z\nb\nc\n"),
                ),
            ],
        )],
    )
    .expect("batch");
    assert_eq!(refusal(&batch), "region-escape");
    assert_eq!(store.tip("alpha"), Some(tip.clone()));
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/b.txt"),
        text("a\nb\nc\n")
    );
}

#[test]
fn rollback_moves_the_pointer_and_keeps_the_commit() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let next = crate::transact::fixture::apply(&mut store, &tip, &actor, "src/a.txt", "new\n");
    let restored = rollback_model(&mut store, "alpha", &next, &tip.head).expect("rollback");
    let Rollback::Restored { tip: current } = restored else {
        panic!("expected restore: {restored:?}");
    };
    assert_eq!(current.head, tip.head);
    assert_eq!(
        store.image_at("alpha", &current.head, "src/a.txt"),
        text("old\n")
    );
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("new\n")
    );
}

#[test]
fn deleting_a_path_removes_it_from_the_new_commit_only() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let batch = transact_facets(
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
                PortableImage::absent(),
            )],
        )],
    )
    .expect("batch");
    let next = applied_tip(&batch);
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        PortableImage::absent()
    );
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
}
