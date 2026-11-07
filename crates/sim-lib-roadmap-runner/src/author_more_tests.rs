// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: a selected source compiles into revision, work, proof, and mutation values.

use std::{collections::BTreeSet, sync::Arc};

use sim_kernel::{Cx, DefaultFactory, HandleSeed, NoopEvalPolicy, Shape};

use super::fixture::{Draft, assemble, deck, happy, spec};
use super::{
    AuthorError, Stance, StatementKind, compile_authoring, diff_members, lint_family, list_family,
    show_member, statement_shape,
};
use crate::{PortableImage, ProofLeaf, SOURCE_SEAL, seal_id};

#[test]
fn statement_shapes_are_distinct() {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x24),
    );
    let advice = statement_shape(StatementKind::Advice);
    let fact = statement_shape(StatementKind::Fact);
    let decision = statement_shape(StatementKind::Decision);
    assert_eq!(
        Shape::is_subshape_of(&advice, &mut cx, &fact).unwrap(),
        Some(false)
    );
    assert_eq!(
        Shape::is_subshape_of(&fact, &mut cx, &decision).unwrap(),
        Some(false)
    );
    assert_eq!(
        Shape::is_subshape_of(&decision, &mut cx, &advice).unwrap(),
        Some(false)
    );
    assert_eq!(
        Shape::is_subshape_of(&advice, &mut cx, &advice).unwrap(),
        Some(true)
    );
    assert_ne!(advice.expected(), fact.expected());
    assert_ne!(fact.expected(), decision.expected());
}

#[test]
fn authority_decisions_refuse() {
    for decision in [
        "begin-authority",
        "begin authority",
        "begin_authority",
        "hold-release",
        "hold release",
        "hold_release",
    ] {
        let consulted = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
            .decision("note the earlier source")
            .bytes();
        let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
            .decision(decision)
            .consult(Some("1"))
            .bytes();
        let family = assemble(&[consulted.clone(), chosen.clone()]);
        let deck = deck(&[
            ("family/consulted.txt", &consulted),
            ("family/chosen.txt", &chosen),
        ]);
        assert_eq!(
            lint_family(&family, &deck, &spec()).unwrap_err(),
            AuthorError::Authority,
            "{decision}"
        );
    }
}

#[test]
fn two_selected_members_refuse() {
    let first = Draft::new("one", "1", "selected", "family/one.txt").bytes();
    let second = Draft::new("two", "2", "selected", "family/two.txt").bytes();
    let family = assemble(&[first.clone(), second.clone()]);
    let deck = deck(&[("family/one.txt", &first), ("family/two.txt", &second)]);
    assert_eq!(
        lint_family(&family, &deck, &spec()).unwrap_err(),
        AuthorError::SelectionCount { count: 2 }
    );
}

#[test]
fn zero_selected_members_refuse() {
    let consulted = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .bytes();
    let family = assemble(std::slice::from_ref(&consulted));
    let deck = deck(&[("family/consulted.txt", &consulted)]);
    assert_eq!(
        lint_family(&family, &deck, &spec()).unwrap_err(),
        AuthorError::SelectionCount { count: 0 }
    );
}

#[test]
fn clean_compile_seals_without_authority() {
    let (family, deck, consulted, chosen) = happy();
    let spec = spec();
    let compiled = compile_authoring(&family, &deck, &spec).unwrap();
    assert_eq!(compiled.source, chosen);
    assert_eq!(compiled.selection.name, "chosen");
    assert_eq!(compiled.selection.round, "2");
    assert_eq!(compiled.selection.source, chosen);
    let source_bytes = compiled.selection.source.as_slice();
    assert!(
        !source_bytes
            .windows(b"begin-authority".len())
            .any(|window| window == b"begin-authority")
    );
    assert!(
        !source_bytes
            .windows(b"hold-release".len())
            .any(|window| window == b"hold-release")
    );
    assert_eq!(compiled.consultation, vec![seal_id(&consulted).unwrap()]);
    assert_eq!(
        compiled
            .revision
            .images()
            .get("family/chosen.txt")
            .map(Vec::as_slice),
        Some(chosen.as_slice())
    );
    assert_eq!(
        compiled
            .revision
            .images()
            .get(SOURCE_SEAL)
            .map(Vec::as_slice),
        Some(chosen.as_slice())
    );
    assert!(
        !compiled
            .revision
            .images()
            .contains_key("family/consulted.txt")
    );
    assert!(!compiled.revision.is_uncertain());
    let claim = &compiled.revision.claims()["semantic-tests"];
    assert!(claim.closed);
    assert_eq!(
        claim.inputs,
        BTreeSet::from(["family/chosen.txt".to_owned()])
    );
    assert_eq!(compiled.work.namespace, "work");
    assert_eq!(compiled.work.phase, "authoring");
    assert_eq!(compiled.work.law, "the work");
    assert_eq!(compiled.work.owner_package, "sim-agent-net");
    assert_eq!(compiled.work.sources[0].body, chosen);
    assert!(!compiled.work.mirrors_agree);
    match &compiled.proof {
        ProofLeaf::SourceDeckPredicate {
            name,
            actual_deck,
            expected_deck,
            required_claims,
            present_claims,
        } => {
            assert_eq!(name, "source-deck");
            assert_eq!(actual_deck, expected_deck);
            assert!(!actual_deck.is_empty());
            assert!(required_claims.contains("semantic-tests"));
            assert!(present_claims.contains("semantic-tests"));
        }
        other => panic!("unexpected proof {other:?}"),
    }
    assert_eq!(compiled.mutation.entries.len(), 1);
    assert_eq!(compiled.mutation.entries[0].path, "family/chosen.txt");
    assert_eq!(
        compiled.mutation.entries[0].preimage,
        PortableImage::absent()
    );
    assert_eq!(
        compiled.mutation.entries[0].postimage,
        PortableImage::file(chosen.clone(), 0o644)
    );
    let rows = list_family(&family, &deck, &spec).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.in_deck && row.roadmap_matches));
    let shown = show_member(&family, "chosen", &deck, &spec).unwrap();
    assert_eq!(shown.member.source, chosen);
    assert_eq!(shown.member.stance, Stance::Selected);
    let diff = diff_members(&family, "earlier", "chosen", &deck, &spec).unwrap();
    assert!(diff.bytes_differ);
    assert!(diff.changed.contains(&"stance"));
    assert!(diff.left_in_deck && diff.right_in_deck);
    let same = diff_members(&family, "chosen", "chosen", &deck, &spec).unwrap();
    assert!(!same.bytes_differ);
    assert!(same.changed.is_empty());
}
