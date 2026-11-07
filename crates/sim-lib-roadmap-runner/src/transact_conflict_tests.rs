// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: each model repository commits or refuses on its own.

use sim_artifact_facet::MergePolicy;

use super::{ArtifactModel, transact_facets};
use crate::transact::fixture::{
    advance, edit, exact, linewise, owner, public_disclosure, raw, refusal, request, seeded, text,
    text_mode,
};

const MODE_BASE: u32 = 0o644;

#[test]
fn exact_overlap_mode_text_and_work_bounds_refuse_without_a_winner() {
    let actor = owner("owner/src");
    let (mut store, base) = seeded("alpha", &[("src/a.txt", "a\n")]);
    let tip = advance(&mut store, "alpha", &base, &actor, "src/a.txt", text("A\n"));
    let exact_conflict = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("a\n"),
                text("B\n"),
            )],
        )],
    )
    .expect("exact");
    assert_eq!(refusal(&exact_conflict), "overlap");
    assert_eq!(store.image_at("alpha", &tip.head, "src/a.txt"), text("A\n"));

    let (mut store, base) = seeded("beta", &[("src/a.txt", "a\n")]);
    let widened = advance(
        &mut store,
        "beta",
        &base,
        &actor,
        "src/a.txt",
        text_mode("a\n", 0o755),
    );
    let mode = transact_facets(
        &mut store,
        &[request(
            "beta",
            &actor,
            &widened,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, linewise()),
                text_mode("a\n", MODE_BASE),
                text_mode("a\n", 0o600),
            )],
        )],
    )
    .expect("mode");
    assert_eq!(refusal(&mode), "conflict");
    assert_eq!(
        store.image_at("beta", &widened.head, "src/a.txt"),
        text_mode("a\n", 0o755)
    );

    let (mut store, base) = seeded("gamma", &[("src/a.txt", "a\n")]);
    let binary = advance(
        &mut store,
        "gamma",
        &base,
        &actor,
        "src/a.txt",
        raw(b"\xff"),
    );
    let text_conflict = transact_facets(
        &mut store,
        &[request(
            "gamma",
            &actor,
            &binary,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, linewise()),
                text("a\n"),
                text("b\n"),
            )],
        )],
    )
    .expect("text");
    assert_eq!(refusal(&text_conflict), "conflict");
    assert_eq!(
        store.image_at("gamma", &binary.head, "src/a.txt"),
        raw(b"\xff")
    );

    let (mut store, base) = seeded("delta", &[("src/a.txt", "a\nb\n")]);
    let tip = advance(
        &mut store,
        "delta",
        &base,
        &actor,
        "src/a.txt",
        text("A\nb\n"),
    );
    let tight = MergePolicy::Linewise {
        max_lines: 1,
        max_cells: 4_096,
    };
    let bounded = transact_facets(
        &mut store,
        &[request(
            "delta",
            &actor,
            &tip,
            &base.head,
            public_disclosure(),
            vec![edit(
                exact("src/a.txt", &actor, tight),
                text("a\nb\n"),
                text("a\nB\n"),
            )],
        )],
    )
    .expect("bound");
    assert_eq!(refusal(&bounded), "conflict");
    assert_eq!(
        store.image_at("delta", &tip.head, "src/a.txt"),
        text("A\nb\n")
    );
}
