// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: each model repository commits or refuses on its own.

use sim_artifact_facet::{
    ArtifactFacet, ArtifactId, FacetError, MergePolicy, ProjectionId, RegionOwnership,
    RegionSelector,
};

use super::{
    ArtifactModel, BoundDisclosure, DisclosureClass, GitModel, RepoOutcome, Rollback,
    rollback_model, transact_facets,
};
use crate::transact::fixture::{
    advance, decision, edit, exact, generated, lines, linewise, owner, public_disclosure, refusal,
    refusal_paths, request, seeded, text,
};

#[test]
fn wrong_head_branch_and_a_missing_repository_refuse() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let mut wrong = request(
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
    );
    wrong.expected_head = "commit/missing".into();
    let batch = transact_facets(&mut store, &[wrong]).expect("head");
    assert_eq!(refusal(&batch), "wrong-head");
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );

    let mut branch = request(
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
    );
    branch.expected_branch = "topic".into();
    let batch = transact_facets(&mut store, &[branch]).expect("branch");
    assert_eq!(refusal(&batch), "wrong-head");

    let missing = request(
        "missing",
        &actor,
        &tip,
        &tip.head,
        public_disclosure(),
        vec![edit(
            exact("src/a.txt", &actor, MergePolicy::Exact),
            text("old\n"),
            text("new\n"),
        )],
    );
    let batch = transact_facets(&mut store, &[missing]).expect("missing");
    assert_eq!(refusal(&batch), "wrong-head");
    assert_eq!(store.tip("alpha"), Some(tip));
}

#[test]
fn dirt_refuses_before_an_otherwise_lawful_apply() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    assert!(store.mark_dirt("alpha", "notes.txt"));
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
                text("new\n"),
            )],
        )],
    )
    .expect("dirt");
    assert_eq!(refusal(&batch), "dirt");
    assert_eq!(refusal_paths(&batch), vec!["notes.txt"]);
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
}

#[test]
fn private_unknown_and_disagreeing_disclosure_refuse() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let edit = edit(
        exact("src/a.txt", &actor, MergePolicy::Exact),
        text("old\n"),
        text("new\n"),
    );
    let private = BoundDisclosure {
        decision: decision("disclosure/model"),
        class: DisclosureClass::Private,
        public_sink: true,
    };
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            private,
            vec![edit.clone()],
        )],
    )
    .expect("private");
    assert_eq!(refusal(&batch), "private-flow");

    let unknown = BoundDisclosure {
        decision: decision("disclosure/model"),
        class: DisclosureClass::Unknown,
        public_sink: true,
    };
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            unknown,
            vec![edit.clone()],
        )],
    )
    .expect("unknown");
    assert_eq!(refusal(&batch), "private-flow");

    let disagree = BoundDisclosure {
        decision: decision("disclosure/other"),
        class: DisclosureClass::Public,
        public_sink: true,
    };
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            disagree,
            vec![edit],
        )],
    )
    .expect("disagree");
    assert_eq!(refusal(&batch), "private-flow");
    assert_eq!(store.tip("alpha"), Some(tip));
}

#[test]
fn a_private_decision_can_commit_when_it_does_not_reach_a_public_sink() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let disclosure = BoundDisclosure {
        decision: decision("disclosure/model"),
        class: DisclosureClass::Private,
        public_sink: false,
    };
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            disclosure,
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("old\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("private");
    assert!(batch.settled());
    let RepoOutcome::Applied {
        tip: private_tip, ..
    } = &batch.outcomes()[0]
    else {
        panic!("expected a commit: {batch:?}");
    };
    let unknown = BoundDisclosure {
        decision: decision("disclosure/model"),
        class: DisclosureClass::Unknown,
        public_sink: false,
    };
    let again = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            private_tip,
            &private_tip.head,
            unknown,
            vec![edit(
                exact("src/a.txt", &actor, MergePolicy::Exact),
                text("new\n"),
                text("newer\n"),
            )],
        )],
    )
    .expect("unknown");
    assert!(again.settled());
    assert!(matches!(again.outcomes()[0], RepoOutcome::Applied { .. }));
}

#[test]
fn a_generated_region_refuses_another_actor_and_admits_its_generator() {
    let generator = owner("owner/gen");
    let other = owner("owner/other");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let spec = generated("src/a.txt", &generator, MergePolicy::Exact);
    let blocked = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &other,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(spec, text("old\n"), text("new\n"))],
        )],
    )
    .expect("bypass");
    assert_eq!(refusal(&blocked), "generated-owner-bypass");
    assert_eq!(store.tip("alpha"), Some(tip.clone()));

    let spec = generated("src/a.txt", &generator, MergePolicy::Exact);
    let allowed = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &generator,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(spec, text("old\n"), text("new\n"))],
        )],
    )
    .expect("generator");
    assert!(allowed.settled());
    let RepoOutcome::Applied { tip: next, .. } = &allowed.outcomes()[0] else {
        panic!("expected a commit: {allowed:?}");
    };
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("new\n")
    );
}

#[test]
fn an_authored_edit_from_another_owner_refuses() {
    let actor = owner("owner/src");
    let other = owner("owner/other");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &other,
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
    .expect("owner");
    assert_eq!(refusal(&batch), "wrong-owner");
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
}

#[test]
fn a_stale_or_unknown_base_refuses_before_merge() {
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
                text("stale\n"),
                text("new\n"),
            )],
        )],
    )
    .expect("stale");
    assert_eq!(refusal(&batch), "stale-base");
    assert_eq!(refusal_paths(&batch), vec!["src/a.txt"]);

    let mut unknown = request(
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
    );
    unknown.base_revision = "commit/missing".into();
    let batch = transact_facets(&mut store, &[unknown]).expect("missing base");
    assert_eq!(refusal(&batch), "stale-base");
    assert_eq!(
        store.image_at("alpha", &tip.head, "src/a.txt"),
        text("old\n")
    );
}

#[test]
fn rollback_refuses_a_wrong_head_dirt_and_an_unknown_commit() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "old\n")]);
    let next = advance(
        &mut store,
        "alpha",
        &tip,
        &actor,
        "src/a.txt",
        text("new\n"),
    );
    let wrong = rollback_model(&mut store, "alpha", &tip, &tip.head).expect("wrong");
    assert!(matches!(
        wrong,
        Rollback::Refused {
            reason: "wrong-head",
            ..
        }
    ));
    assert_eq!(store.tip("alpha"), Some(next.clone()));

    let unknown = rollback_model(&mut store, "alpha", &next, "commit/missing").expect("unknown");
    assert!(matches!(
        unknown,
        Rollback::Refused {
            reason: "wrong-head",
            ..
        }
    ));
    assert_eq!(store.tip("alpha"), Some(next.clone()));

    assert!(store.mark_dirt("alpha", "notes.txt"));
    let dirty = rollback_model(&mut store, "alpha", &next, &tip.head).expect("dirt");
    assert!(matches!(dirty, Rollback::Refused { reason: "dirt", .. }));
    assert_eq!(store.tip("alpha"), Some(next));
}

#[test]
fn the_facet_law_rejects_a_path_escape_and_a_generated_owner_mismatch() {
    let actor = owner("owner/src");
    let other = owner("owner/other");
    let escaped = ArtifactFacet::new(
        ArtifactId::from_text("artifact/escape").expect("artifact"),
        actor.clone(),
        RegionSelector::ExactPath("../outside".into()),
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        decision("disclosure/model"),
        RegionOwnership::Authored,
    );
    assert!(matches!(escaped, Err(FacetError::InvalidRegion)));

    let bypass = ArtifactFacet::new(
        ArtifactId::from_text("artifact/generated").expect("artifact"),
        actor,
        RegionSelector::ExactPath("src/a.txt".into()),
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        decision("disclosure/model"),
        RegionOwnership::Generated { generator: other },
    );
    assert!(matches!(bypass, Err(FacetError::WrongOwner)));
}

#[test]
fn a_line_range_that_stays_inside_its_region_commits() {
    let actor = owner("owner/src");
    let (mut store, tip) = seeded("alpha", &[("src/a.txt", "a\nb\nc\n")]);
    let batch = transact_facets(
        &mut store,
        &[request(
            "alpha",
            &actor,
            &tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                lines("src/a.txt", &actor, 1, 2, linewise()),
                text("a\nb\nc\n"),
                text("a\nB\nc\n"),
            )],
        )],
    )
    .expect("range");
    assert!(batch.settled());
    let RepoOutcome::Applied { tip: next, .. } = &batch.outcomes()[0] else {
        panic!("expected a commit: {batch:?}");
    };
    assert_eq!(
        store.image_at("alpha", &next.head, "src/a.txt"),
        text("a\nB\nc\n")
    );
}
