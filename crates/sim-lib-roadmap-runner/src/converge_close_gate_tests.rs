// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;

use sim_kernel::{ContentId, Symbol};

use super::close_fixture::{
    attachment, canon, catalog, closure_of, command_leaf, commit_both, deck_leaf, equality,
    failed_equality, open_journal, proof_line, recorded_proofs, reopen, revision_with, submit,
};
use super::fixture::{complete_fence, opened};
use super::{AttachStep, Fence, Step};

fn refused(step: AttachStep) -> &'static str {
    match step {
        AttachStep::Refused { reason, .. } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn stale_or_uncertain_or_empty_calls_journal_nothing() {
    let mut revision = opened();
    let id = revision.id.clone();
    let (journal, head) = open_journal();
    let bogus = ContentId::from_bytes(Symbol::qualified("core", "sha256"), [9; 32]);
    let leaves = catalog([equality("leaf/x", b"x")]);
    let one = [attachment("claim/alpha", "leaf/x")];
    let stale = super::attach_proofs(&mut revision, &leaves, &bogus, &journal, &head, &one)
        .expect("stale fence");
    assert_eq!(refused(stale), "stale-revision");
    assert_eq!(revision.id, id);

    let mut revision = opened();
    let id = revision.id.clone();
    let images = revision.images().clone();
    let held = super::propose(
        &mut revision,
        complete_fence("fact/alpha", b"alpha-1", Fence::BeforeCommit),
    )
    .expect("hold");
    assert!(matches!(held, Step::Held { .. }));
    assert!(revision.is_uncertain());
    assert_eq!(revision.images(), &images);
    assert!(revision.claims["claim/alpha"].closed);
    let (journal, head) = open_journal();
    let uncertain = submit(&mut revision, &leaves, &journal, &head, &one).expect("uncertain");
    assert_eq!(refused(uncertain), "uncertain");
    assert_eq!(revision.id, id);
    assert_eq!(revision.images(), &images);

    let mut revision = opened();
    let id = revision.id.clone();
    let (journal, head) = open_journal();
    assert_eq!(
        refused(submit(&mut revision, &leaves, &journal, &head, &[]).expect("empty")),
        "empty-attachment"
    );
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[attachment("", "leaf/x")]
            )
            .expect("blank claim")
        ),
        "empty-attachment"
    );
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[attachment("claim/alpha", "")]
            )
            .expect("blank proof")
        ),
        "empty-attachment"
    );
    assert_eq!(revision.id, id);
    assert!(recorded_proofs(&journal).is_empty());
}

#[test]
fn structural_mismatches_refuse_the_whole_batch() {
    let mut revision = opened();
    let id = revision.id.clone();
    let (journal, head) = open_journal();
    let leaves = catalog([equality("leaf/a", b"a"), equality("leaf/b", b"b")]);
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[attachment("claim/alpha", "leaf/a")]
            )
            .expect("closed")
        ),
        "already-closed"
    );
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[attachment("claim/missing", "leaf/a")]
            )
            .expect("missing claim")
        ),
        "unknown-claim"
    );

    let affected = reopen(&mut revision, "fact/alpha", b"alpha-1");
    assert_eq!(affected, BTreeSet::from(["claim/alpha".to_owned()]));
    let fenced = revision.id.clone();
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[
                    attachment("claim/alpha", "leaf/missing"),
                    attachment("claim/beta", "leaf/b"),
                ]
            )
            .expect("missing leaf")
        ),
        "unknown-proof"
    );
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[
                    attachment("claim/alpha", "leaf/a"),
                    attachment("claim/alpha", "leaf/b"),
                ]
            )
            .expect("duplicate")
        ),
        "duplicate-claim"
    );
    assert_eq!(
        refused(
            submit(
                &mut revision,
                &leaves,
                &journal,
                &head,
                &[
                    attachment("claim/beta", "leaf/b"),
                    attachment("claim/alpha", "leaf/a"),
                ]
            )
            .expect("closed sibling")
        ),
        "already-closed"
    );
    assert_eq!(revision.id, fenced);
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(revision.claims["claim/beta"].closed);
    assert_ne!(revision.id, id);
    assert!(recorded_proofs(&journal).is_empty());
}

#[test]
fn only_the_current_input_closure_closes() {
    let mut revision = opened();
    let old = closure_of(&revision, "claim/alpha");
    let other = closure_of(&revision, "claim/seal");
    reopen(&mut revision, "fact/alpha", b"alpha-1");
    let fenced = revision.id.clone();
    let current = closure_of(&revision, "claim/alpha");
    assert_ne!(old, current);
    assert_ne!(other, current);
    assert_ne!(b"alpha-1".as_slice(), current.as_slice());
    let leaves = catalog([
        equality("leaf/old", &old),
        equality("leaf/other", &other),
        equality("leaf/raw", b"alpha-1"),
        failed_equality("leaf/fail", &current, b"no"),
        equality("leaf/now", &current),
    ]);
    let (journal, mut head) = open_journal();
    for (leaf, reason) in [
        ("leaf/old", "stale-inputs"),
        ("leaf/other", "stale-inputs"),
        ("leaf/raw", "stale-inputs"),
        ("leaf/fail", "proof-failed"),
    ] {
        let step = submit(
            &mut revision,
            &leaves,
            &journal,
            &head,
            &[attachment("claim/alpha", leaf)],
        )
        .expect(leaf);
        let AttachStep::Open {
            reasons,
            revision: id,
            head: next,
            ..
        } = step
        else {
            panic!("{leaf} closed a claim");
        };
        assert_eq!(reasons.get("claim/alpha"), Some(&reason));
        assert_eq!(id, fenced);
        assert!(!revision.claims["claim/alpha"].closed);
        head = next;
    }
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/alpha", "leaf/now")],
    )
    .expect("close");
    let AttachStep::Closed { closed, head, .. } = step else {
        panic!("current closure should close");
    };
    assert_eq!(closed, BTreeSet::from(["claim/alpha".to_owned()]));
    assert!(revision.claims["claim/alpha"].closed);
    assert_ne!(revision.id, fenced);
    let again = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/alpha", "leaf/now")],
    )
    .expect("second");
    assert_eq!(refused(again), "already-closed");
    let proofs = recorded_proofs(&journal);
    assert_eq!(proofs.len(), 5);
    assert!(proofs[..4].iter().all(|(_, passed)| !passed));
    assert_eq!(
        proofs[4],
        (proof_line(&fenced, "claim/alpha", "leaf/now"), true)
    );
}

#[test]
fn length_prefixes_reject_a_concat_collision() {
    let glued = |path: &str, bytes: &[u8]| {
        let mut out = path.as_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    };
    assert_eq!(glued("fact/ab", b"c"), glued("fact/a", b"bc"));
    assert_ne!(canon("fact/ab", b"c"), canon("fact/a", b"bc"));

    let mut revision = revision_with("fact/ab", b"c0", "claim/ab");
    reopen(&mut revision, "fact/ab", b"c");
    let fenced = revision.id.clone();
    let wrong = canon("fact/a", b"bc");
    let right = closure_of(&revision, "claim/ab");
    let leaves = catalog([
        equality("leaf/wrong", &wrong),
        equality("leaf/right", &right),
    ]);
    let (journal, head) = open_journal();
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/ab", "leaf/wrong")],
    )
    .expect("wrong pairing");
    let AttachStep::Open { reasons, head, .. } = step else {
        panic!("colliding bytes must stay open");
    };
    assert_eq!(reasons.get("claim/ab"), Some(&"stale-inputs"));
    assert_eq!(revision.id, fenced);
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/ab", "leaf/right")],
    )
    .expect("right");
    assert!(matches!(step, AttachStep::Closed { .. }));
    assert!(revision.claims["claim/ab"].closed);
}

#[test]
fn command_and_source_deck_do_not_close() {
    let mut revision = opened();
    reopen(&mut revision, "fact/alpha", b"alpha-1");
    let current = closure_of(&revision, "claim/alpha");
    let leaves = catalog([
        command_leaf("leaf/command"),
        deck_leaf("leaf/deck"),
        equality("leaf/now", &current),
    ]);
    let (journal, mut head) = open_journal();
    for leaf in ["leaf/command", "leaf/deck"] {
        let step = submit(
            &mut revision,
            &leaves,
            &journal,
            &head,
            &[attachment("claim/alpha", leaf)],
        )
        .expect(leaf);
        let AttachStep::Open {
            reasons,
            head: next,
            ..
        } = step
        else {
            panic!("{leaf} closed a claim without checking its inputs");
        };
        assert_eq!(reasons.get("claim/alpha"), Some(&"unchecked-inputs"));
        assert!(!revision.claims["claim/alpha"].closed);
        head = next;
    }
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/alpha", "leaf/now")],
    )
    .expect("close");
    assert!(matches!(step, AttachStep::Closed { .. }));
    assert!(revision.claims["claim/alpha"].closed);
    let proofs = recorded_proofs(&journal);
    assert_eq!(proofs.len(), 3);
    assert!(!proofs[0].1 && !proofs[1].1 && proofs[2].1);
}

#[test]
fn a_batch_closes_only_claims_whose_current_inputs_were_checked() {
    let mut revision = opened();
    let affected = commit_both(&mut revision);
    assert_eq!(
        affected,
        BTreeSet::from(["claim/alpha".to_owned(), "claim/beta".to_owned()])
    );
    let fenced = revision.id.clone();
    let alpha = closure_of(&revision, "claim/alpha");
    let beta = closure_of(&revision, "claim/beta");
    let leaves = catalog([
        equality("leaf/alpha", &alpha),
        failed_equality("leaf/beta", &beta, b"no"),
    ]);
    let (journal, head) = open_journal();
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[
            attachment("claim/beta", "leaf/beta"),
            attachment("claim/alpha", "leaf/alpha"),
        ],
    )
    .expect("partial");
    let AttachStep::Closed {
        closed,
        open,
        reasons,
        ..
    } = step
    else {
        panic!("one checked claim should close");
    };
    assert_eq!(closed, BTreeSet::from(["claim/alpha".to_owned()]));
    assert_eq!(open, BTreeSet::from(["claim/beta".to_owned()]));
    assert_eq!(reasons.get("claim/beta"), Some(&"proof-failed"));
    assert!(!reasons.contains_key("claim/alpha"));
    assert!(revision.claims["claim/alpha"].closed);
    assert!(!revision.claims["claim/beta"].closed);
    assert!(revision.claims["claim/seal"].closed);
    assert_ne!(revision.id, fenced);
    assert_eq!(
        recorded_proofs(&journal),
        vec![
            (proof_line(&fenced, "claim/alpha", "leaf/alpha"), true),
            (proof_line(&fenced, "claim/beta", "leaf/beta"), false),
        ]
    );

    let mut revision = opened();
    commit_both(&mut revision);
    let alpha = closure_of(&revision, "claim/alpha");
    let leaves = catalog([equality("leaf/alpha", &alpha)]);
    let (journal, head) = open_journal();
    let step = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/alpha", "leaf/alpha")],
    )
    .expect("one");
    let AttachStep::Closed {
        closed,
        open,
        reasons,
        ..
    } = step
    else {
        panic!("the checked claim should close");
    };
    assert_eq!(closed, BTreeSet::from(["claim/alpha".to_owned()]));
    assert_eq!(open, BTreeSet::from(["claim/beta".to_owned()]));
    assert!(reasons.is_empty());
    assert!(!revision.claims["claim/beta"].closed);
    assert!(revision.claims["claim/beta"].digest.is_none());
    assert_eq!(recorded_proofs(&journal).len(), 1);
}

#[test]
fn a_missing_input_stays_open_while_a_checked_sibling_closes() {
    let mut revision = opened();
    commit_both(&mut revision);
    assert!(revision.images.remove("fact/alpha").is_some());
    let beta = closure_of(&revision, "claim/beta");
    let fence = super::close_body::accepted_id(&revision).expect("image without alpha");
    let leaves = catalog([equality("leaf/alpha", b"x"), equality("leaf/beta", &beta)]);
    let (journal, head) = open_journal();
    let step = super::attach_proofs(
        &mut revision,
        &leaves,
        &fence,
        &journal,
        &head,
        &[
            attachment("claim/alpha", "leaf/alpha"),
            attachment("claim/beta", "leaf/beta"),
        ],
    )
    .expect("partial");
    let AttachStep::Closed {
        closed,
        open,
        reasons,
        ..
    } = step
    else {
        panic!("the sibling with bytes present should close");
    };
    assert_eq!(closed, BTreeSet::from(["claim/beta".to_owned()]));
    assert_eq!(open, BTreeSet::from(["claim/alpha".to_owned()]));
    assert_eq!(reasons.get("claim/alpha"), Some(&"unknown-input"));
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(revision.claims["claim/beta"].closed);
}
