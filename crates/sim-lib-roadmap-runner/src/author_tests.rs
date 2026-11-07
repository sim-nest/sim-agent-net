// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: a selected source compiles into revision, work, proof, and mutation values.

use super::fixture::{Draft, assemble, bare, deck, happy, spec, spec_checkpoint, without};
use super::{
    AuthorError, Family, Stance, compile_authoring, lint_family, parse_member, source_round_trip,
};

#[test]
fn source_round_trips_exactly() {
    let bytes = Draft::new("chosen", "2", "selected", "family/chosen.txt").bytes();
    let member = parse_member(&bytes).unwrap();
    assert_eq!(source_round_trip(&member), bytes.as_slice());
    assert_eq!(
        parse_member(source_round_trip(&member)).unwrap().source,
        bytes
    );
}

#[test]
fn missing_sections_refuse() {
    for section in ["argument", "work", "gate", "ratchet", "owner", "checker"] {
        let error = parse_member(&without(section)).unwrap_err();
        assert_eq!(error, AuthorError::Missing { section });
    }
}

#[test]
fn consulted_sibling_does_not_satisfy() {
    let consulted = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .bytes();
    let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .fact(None)
        .consult(Some("1"))
        .bytes();
    let family = assemble(&[consulted.clone(), chosen.clone()]);
    let deck = deck(&[
        ("family/consulted.txt", &consulted),
        ("family/chosen.txt", &chosen),
    ]);
    let error = lint_family(&family, &deck, &spec()).unwrap_err();
    assert_eq!(
        error,
        AuthorError::Unsatisfied {
            requirement: "semantic-tests".into(),
        }
    );
}

#[test]
fn superseded_sibling_does_not_satisfy() {
    let old = Draft::new("old", "1", "superseded", "family/old.txt")
        .decision("note the replaced source")
        .bytes();
    let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .fact(None)
        .bytes();
    let family = assemble(&[old.clone(), chosen.clone()]);
    let deck = deck(&[("family/old.txt", &old), ("family/chosen.txt", &chosen)]);
    let error = lint_family(&family, &deck, &spec()).unwrap_err();
    assert_eq!(
        error,
        AuthorError::Unsatisfied {
            requirement: "semantic-tests".into(),
        }
    );
}

#[test]
fn deck_bytes_mismatch_refuses() {
    let (family, _, consulted, chosen) = happy();
    let mut flipped = chosen.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x01;
    let deck = deck(&[
        ("family/consulted.txt", &consulted),
        ("family/chosen.txt", &flipped),
    ]);
    let error = lint_family(&family, &deck, &spec()).unwrap_err();
    assert_eq!(
        error,
        AuthorError::DeckMismatch {
            path: "family/chosen.txt".into(),
        }
    );
}

#[test]
fn roadmap_id_mismatch_refuses() {
    let consulted = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .bytes();
    let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .roadmap("other-roadmap")
        .consult(Some("1"))
        .bytes();
    let family = assemble(&[consulted.clone(), chosen.clone()]);
    let deck = deck(&[
        ("family/consulted.txt", &consulted),
        ("family/chosen.txt", &chosen),
    ]);
    let error = lint_family(&family, &deck, &spec()).unwrap_err();
    assert_eq!(
        error,
        AuthorError::RoadmapMismatch {
            found: "other-roadmap".into(),
            spec: "value-roadmap".into(),
        }
    );
}

#[test]
fn fact_outside_claim_grammar_refuses() {
    for fact in ["needs review", "*", "a+b"] {
        let bytes = Draft::new("chosen", "2", "selected", "family/chosen.txt")
            .fact(Some(fact))
            .bytes();
        assert_eq!(
            parse_member(&bytes).unwrap_err(),
            AuthorError::Identity { kind: "fact" },
            "{fact}"
        );
    }
}

#[test]
fn sibling_consult_must_name_consultation() {
    let missing = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .consult(Some("9"))
        .bytes();
    let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .consult(Some("1"))
        .bytes();
    let family = assemble(&[missing.clone(), chosen.clone()]);
    let missing_deck = deck(&[
        ("family/consulted.txt", &missing),
        ("family/chosen.txt", &chosen),
    ]);
    assert_eq!(
        lint_family(&family, &missing_deck, &spec()).unwrap_err(),
        AuthorError::Consult { round: "9".into() }
    );

    let selected_round = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .consult(Some("2"))
        .bytes();
    let later = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .consult(Some("1"))
        .bytes();
    let family = assemble(&[selected_round.clone(), later.clone()]);
    let selected_deck = deck(&[
        ("family/consulted.txt", &selected_round),
        ("family/chosen.txt", &later),
    ]);
    assert_eq!(
        lint_family(&family, &selected_deck, &spec()).unwrap_err(),
        AuthorError::Consult { round: "2".into() }
    );
}

#[test]
fn reserved_and_long_paths_refuse() {
    let long = "a".repeat(4097);
    let tabbed = "family/a\tb";
    for path in [
        "source-seal",
        "source-seal/member.txt",
        long.as_str(),
        tabbed,
    ] {
        let bytes = Draft::new("chosen", "2", "selected", path).bytes();
        assert_eq!(
            parse_member(&bytes).unwrap_err(),
            AuthorError::Path,
            "{path}"
        );
    }
}

#[test]
fn owner_and_roadmap_surfaces_refuse() {
    let dotted_owner = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .owner("foo..bar")
        .bytes();
    assert_eq!(
        parse_member(&dotted_owner).unwrap_err(),
        AuthorError::Identity { kind: "owner" }
    );
    let dotted_roadmap = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .roadmap("foo..bar")
        .bytes();
    assert_eq!(
        parse_member(&dotted_roadmap).unwrap_err(),
        AuthorError::Identity { kind: "roadmap" }
    );
    let same = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .owner("value-roadmap")
        .bytes();
    assert_eq!(
        parse_member(&same).unwrap_err(),
        AuthorError::Identity { kind: "owner" }
    );

    let consulted = bare("family/consulted.txt", Stance::Consulted, "1");
    let mut selected = bare("family/chosen.txt", Stance::Selected, "2");
    selected.owner = "foo..bar".to_owned();
    let family = Family::from_members(vec![consulted.clone(), selected.clone()]).unwrap();
    let deck = deck(&[
        ("family/consulted.txt", consulted.source.as_slice()),
        ("family/chosen.txt", selected.source.as_slice()),
    ]);
    let spec = spec();
    assert_eq!(
        lint_family(&family, &deck, &spec).unwrap_err(),
        AuthorError::Identity { kind: "owner" }
    );
    assert_eq!(
        compile_authoring(&family, &deck, &spec).unwrap_err(),
        AuthorError::Identity { kind: "owner" }
    );
}

#[test]
fn reserved_path_refuses_before_revision() {
    let consulted = bare("family/consulted.txt", Stance::Consulted, "1");
    let selected = bare("source-seal/member.txt", Stance::Selected, "2");
    let family = Family::from_members(vec![consulted.clone(), selected.clone()]).unwrap();
    let sealed_deck = deck(&[
        ("family/consulted.txt", consulted.source.as_slice()),
        ("source-seal/member.txt", selected.source.as_slice()),
    ]);
    let spec = spec();
    assert_eq!(
        lint_family(&family, &sealed_deck, &spec).unwrap_err(),
        AuthorError::Path
    );
    assert_eq!(
        compile_authoring(&family, &sealed_deck, &spec).unwrap_err(),
        AuthorError::Path
    );

    let consulted = bare("family/consulted.txt", Stance::Consulted, "1");
    let selected = bare("family/a\tb", Stance::Selected, "2");
    let family = Family::from_members(vec![consulted.clone(), selected.clone()]).unwrap();
    let tabbed_deck = deck(&[
        ("family/consulted.txt", consulted.source.as_slice()),
        ("family/a\tb", selected.source.as_slice()),
    ]);
    assert_eq!(
        lint_family(&family, &tabbed_deck, &spec).unwrap_err(),
        AuthorError::Path
    );
    assert_eq!(
        compile_authoring(&family, &tabbed_deck, &spec).unwrap_err(),
        AuthorError::Path
    );
}

#[test]
fn inadmissible_fact_refuses_before_revision() {
    let consulted = bare("family/consulted.txt", Stance::Consulted, "1");
    let mut selected = bare("family/chosen.txt", Stance::Selected, "2");
    selected.facts.push("needs review".to_owned());
    let family = Family::from_members(vec![consulted.clone(), selected.clone()]).unwrap();
    let deck = deck(&[
        ("family/consulted.txt", consulted.source.as_slice()),
        ("family/chosen.txt", selected.source.as_slice()),
    ]);
    let spec = spec();
    assert_eq!(
        lint_family(&family, &deck, &spec).unwrap_err(),
        AuthorError::Identity { kind: "fact" }
    );
    assert_eq!(
        compile_authoring(&family, &deck, &spec).unwrap_err(),
        AuthorError::Identity { kind: "fact" }
    );

    let (family, deck, _, _) = happy();
    let error = lint_family(&family, &deck, &spec_checkpoint("needs review")).unwrap_err();
    assert_eq!(error, AuthorError::Identity { kind: "fact" });
}
