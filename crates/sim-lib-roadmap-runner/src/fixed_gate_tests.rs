// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: sealed fixed point.

use std::collections::BTreeSet;

use sim_artifact_facet::{
    ArtifactFacet, ArtifactId, DisclosureDecisionId, FacetError, MergePolicy, OwnerId,
    ProjectionId, RegionOwnership, RegionSelector,
};

use super::fixture::{Gen, World, calls, facet, paths, refusal, seal, world};
use super::{FactGenerator, seal_fixed_point};
use crate::schedule::CLASS_PROOF_FAILED;
use crate::{
    Fence, OpenClaim, ProofOutcome, Proposal, ProposalBody, RevisionOpen, SOURCE_SEAL, Step,
    open_revision, propose, seal_id,
};

#[test]
fn failed_and_missing_proofs_block_before_generation() {
    let mut world = world();
    let accepted = world.revision.clone();
    world
        .proofs
        .insert("claim/alpha".to_owned(), ProofOutcome::Failed);
    world
        .proofs
        .insert("claim/beta".to_owned(), ProofOutcome::Failed);
    let failed = seal(&world, &BTreeSet::new(), None).expect("failed");
    assert_eq!(
        failed,
        refusal(CLASS_PROOF_FAILED, &["claim/alpha", "claim/beta"])
    );
    assert_eq!(calls(&world), 0);
    assert_eq!(world.revision, accepted);

    let mut missing_world = super::fixture::world();
    missing_world.proofs.remove("claim/src");
    let missing = seal(&missing_world, &BTreeSet::new(), None).expect("missing");
    assert_eq!(missing, refusal("proof-gap", &["claim/src"]));
    assert_eq!(calls(&missing_world), 0);

    let mut both_world = super::fixture::world();
    both_world
        .proofs
        .insert("claim/alpha".to_owned(), ProofOutcome::Failed);
    both_world.proofs.remove("claim/src");
    let both = seal(&both_world, &BTreeSet::new(), None).expect("both");
    assert_eq!(
        both,
        refusal(CLASS_PROOF_FAILED, &["claim/alpha", "claim/src"])
    );
    assert_eq!(calls(&both_world), 0);
}

#[test]
fn uncertain_and_open_claims_block_before_generation() {
    let mut world = world();
    let held = propose(
        &mut world.revision,
        change(Fence::BeforeCommit, "fact/src", b"src-2"),
    );
    assert!(matches!(held, Ok(Step::Held { .. })));
    assert!(world.revision.is_uncertain());
    let accepted = world.revision.clone();
    let step = seal(&world, &BTreeSet::new(), None).expect("uncertain");
    assert_eq!(step, refusal("uncertain", &[]));
    assert_eq!(world.revision, accepted);
    assert_eq!(calls(&world), 0);

    let mut opened = super::fixture::world();
    let committed = propose(
        &mut opened.revision,
        change(Fence::Commit, "fact/alpha", b"alpha-b"),
    );
    let Ok(Step::Invalidated { affected, .. }) = committed else {
        panic!("expected the claim to open");
    };
    assert!(affected.contains("claim/alpha"));
    assert!(opened.revision.claims()["claim/beta"].closed);
    let accepted = opened.revision.clone();
    let step = seal(&opened, &BTreeSet::new(), None).expect("open");
    assert_eq!(step, refusal("open-claim", &["claim/alpha"]));
    assert_eq!(opened.revision, accepted);
    assert_eq!(calls(&opened), 0);
}

#[test]
fn duplicate_split_cycle_and_authored_writes_do_not_generate() {
    let world = world();
    let accepted = world.revision.clone();
    let duplicate = world.alpha.twin();
    let generators: [&dyn FactGenerator; 3] = [&world.alpha, &world.beta, &duplicate];
    let step = seal_fixed_point(
        &world.revision,
        &world.facets,
        &generators,
        &world.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("duplicate");
    assert_eq!(
        step,
        refusal("duplicate-generator", &["fact/alpha", "fact/extra"])
    );
    assert_eq!(calls(&world), 0);
    assert_eq!(duplicate.calls.get(), 0);

    let mut facets = world.facets.clone();
    facets[2] = facet("artifact/alpha", &world.beta.owner, "fact/extra", true);
    let generators: [&dyn FactGenerator; 2] = [&world.alpha, &world.beta];
    let step = seal_fixed_point(
        &world.revision,
        &facets,
        &generators,
        &world.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("split");
    assert_eq!(
        step,
        refusal("split-generator", &["fact/alpha", "fact/extra"])
    );
    assert_eq!(calls(&world), 0);

    let cyclic = Gen::new(
        &world.alpha.owner,
        &["fact/alpha"],
        &[("fact/alpha", b"alpha")],
    );
    let generators: [&dyn FactGenerator; 1] = [&cyclic];
    let step = seal_fixed_point(
        &world.revision,
        &world.facets,
        &generators,
        &world.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("cycle");
    assert_eq!(step, refusal("generator-cycle", &["fact/alpha"]));
    assert_eq!(cyclic.calls.get(), 0);

    let authored = Gen::new(&world.alpha.owner, &["fact/src"], &[("fact/src", b"src")]);
    let generators: [&dyn FactGenerator; 1] = [&authored];
    let step = seal_fixed_point(
        &world.revision,
        &world.facets,
        &generators,
        &world.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("authored");
    assert_eq!(step, refusal("authored-write", &["fact/src"]));
    assert_eq!(authored.calls.get(), 0);
    assert_eq!(world.revision, accepted);
}

#[test]
fn region_grain_and_deletion_of_an_ungenerated_path_block() {
    let world = world();
    let accepted = world.revision.clone();
    let mut facets = world.facets.clone();
    facets.push(line_facet("fact/alpha"));
    let generators: [&dyn FactGenerator; 2] = [&world.alpha, &world.beta];
    let step = seal_fixed_point(
        &world.revision,
        &facets,
        &generators,
        &world.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("line");
    assert_eq!(step, refusal("region-grain", &["fact/alpha"]));
    assert_eq!(calls(&world), 0);

    let unknown = seal(&world, &paths(&["fact/missing"]), None).expect("unknown");
    assert_eq!(unknown, refusal("unknown-fact", &["fact/missing"]));
    let authored = seal(&world, &paths(&["fact/src"]), None).expect("authored");
    assert_eq!(authored, refusal("not-generated", &["fact/src"]));
    assert_eq!(calls(&world), 0);
    assert_eq!(world.revision, accepted);
}

#[test]
fn wrong_owner_is_the_facet_constructors_refusal() {
    let authored = OwnerId::from_text("owner/authored").expect("owner");
    let generator = OwnerId::from_text("owner/generator").expect("owner");
    assert_ne!(authored, generator);
    let built = ArtifactFacet::new(
        ArtifactId::from_text("artifact/one").expect("artifact"),
        authored,
        RegionSelector::ExactPath("fact/alpha".to_owned()),
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        DisclosureDecisionId::from_text("disclosure/model").expect("disclosure"),
        RegionOwnership::Generated { generator },
    );
    assert_eq!(built, Err(FacetError::WrongOwner));
}

#[test]
fn a_present_unfaceted_write_is_an_inexact_generator() {
    let base = world();
    let accepted = base.revision.clone();
    let seal_bytes = base
        .revision
        .images()
        .get(SOURCE_SEAL)
        .expect("seal")
        .clone();
    let mut images = base.revision.images().clone();
    images.insert("fact/notes".to_owned(), b"notes".to_vec());
    let claims = base
        .revision
        .claims()
        .iter()
        .map(|(id, claim)| OpenClaim {
            id: id.clone(),
            inputs: claim.inputs.clone(),
        })
        .collect();
    let revision = open_revision(RevisionOpen {
        seal_id: seal_id(&seal_bytes).expect("id"),
        seal: seal_bytes,
        images,
        claims,
    })
    .expect("revision");
    let notes = Gen::new(
        &base.alpha.owner,
        &["fact/src"],
        &[("fact/alpha", b"alpha"), ("fact/notes", b"notes")],
    );
    let generators: [&dyn FactGenerator; 2] = [&notes, &base.beta];
    let step = seal_fixed_point(
        &revision,
        &base.facets,
        &generators,
        &base.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("present");
    assert_eq!(
        step,
        refusal("split-generator", &["fact/extra", "fact/notes"])
    );
    assert_eq!(notes.calls.get(), 0);

    let absent = Gen::new(
        &base.alpha.owner,
        &["fact/src"],
        &[("fact/alpha", b"alpha"), ("fact/missing", b"missing")],
    );
    let generators: [&dyn FactGenerator; 2] = [&absent, &base.beta];
    let step = seal_fixed_point(
        &base.revision,
        &base.facets,
        &generators,
        &base.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("absent");
    assert_eq!(step, refusal("unknown-fact", &["fact/missing"]));
    assert_eq!(absent.calls.get(), 0);
    assert_eq!(base.beta.calls.get(), 0);
    assert_eq!(base.revision, accepted);
}

#[test]
fn an_absent_faceted_write_is_unknown_before_an_inexact_set() {
    let base = world();
    let accepted = base.revision.clone();
    let hidden = revision_without(&base, &["fact/extra"], &[("fact/notes", b"notes")]);
    let mixed = Gen::new(
        &base.alpha.owner,
        &["fact/src"],
        &[("fact/extra", b"extra"), ("fact/notes", b"notes")],
    );
    let generators: [&dyn FactGenerator; 2] = [&mixed, &base.beta];
    let step = seal_fixed_point(
        &hidden,
        &base.facets,
        &generators,
        &base.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("hidden");
    assert_eq!(step, refusal("unknown-fact", &["fact/extra"]));
    assert_eq!(mixed.calls.get(), 0);

    let both = revision_without(&base, &["fact/alpha", "fact/extra"], &[]);
    let exact = Gen::new(
        &base.alpha.owner,
        &["fact/src"],
        &[("fact/alpha", b"alpha"), ("fact/extra", b"extra")],
    );
    let generators: [&dyn FactGenerator; 2] = [&exact, &base.beta];
    let step = seal_fixed_point(
        &both,
        &base.facets,
        &generators,
        &base.proofs,
        &BTreeSet::new(),
        None,
    )
    .expect("both");
    assert_eq!(step, refusal("unknown-fact", &["fact/alpha", "fact/extra"]));
    assert_eq!(exact.calls.get(), 0);
    assert_eq!(base.beta.calls.get(), 0);
    assert_eq!(base.revision, accepted);
}

fn revision_without(base: &World, drop: &[&str], add: &[(&str, &[u8])]) -> crate::Revision {
    let seal_bytes = base
        .revision
        .images()
        .get(SOURCE_SEAL)
        .expect("seal")
        .clone();
    let mut images = base.revision.images().clone();
    for path in drop {
        images.remove(*path);
    }
    for (path, bytes) in add {
        images.insert((*path).to_owned(), (*bytes).to_vec());
    }
    let claims = base
        .revision
        .claims()
        .iter()
        .filter(|(_, claim)| claim.inputs.iter().all(|input| images.contains_key(input)))
        .map(|(id, claim)| OpenClaim {
            id: id.clone(),
            inputs: claim.inputs.clone(),
        })
        .collect();
    open_revision(RevisionOpen {
        seal_id: seal_id(&seal_bytes).expect("id"),
        seal: seal_bytes,
        images,
        claims,
    })
    .expect("revision")
}

fn change(fence: Fence, path: &str, bytes: &[u8]) -> Proposal {
    let mut facts = std::collections::BTreeMap::new();
    facts.insert(path.to_owned(), bytes.to_vec());
    Proposal {
        author: "author".to_owned(),
        checker: "checker".to_owned(),
        fence,
        body: ProposalBody::Complete {
            id: "proposal/1".to_owned(),
            facts,
            limit: 100,
        },
    }
}

fn line_facet(path: &str) -> ArtifactFacet {
    let owner = OwnerId::from_text("owner/line").expect("owner");
    ArtifactFacet::new(
        ArtifactId::from_text("artifact/line").expect("artifact"),
        owner.clone(),
        RegionSelector::LineRange {
            path: path.to_owned(),
            start: 0,
            end: 1,
        },
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        DisclosureDecisionId::from_text("disclosure/model").expect("disclosure"),
        RegionOwnership::Generated { generator: owner },
    )
    .expect("line facet")
}
