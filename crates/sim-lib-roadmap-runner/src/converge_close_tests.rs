// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::{
    ArtifactFacet, ArtifactId, DisclosureDecisionId, MergePolicy, OwnerId, ProjectionId,
    RegionOwnership, RegionSelector,
};
use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::JournalError;

use super::close_fixture::{
    attachment, catalog, closure_of, equality, open_journal, proof_line, recorded_proofs, reopen,
    submit,
};
use super::fixture::{claim, complete, malformed, opened};
use super::{OpenRequest, SOURCE_SEAL, Step, open, seal_id};
use crate::{
    ExecutionJournalError, ExecutionRecord, FactGenerator, FixedSeal, FixedStep, ProofOutcome,
    seal_fixed_point,
};

struct Gen {
    owner: OwnerId,
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    calls: Cell<u32>,
    seen: RefCell<Vec<u8>>,
}

impl Gen {
    fn new(owner: OwnerId) -> Self {
        Self {
            owner,
            reads: BTreeSet::from(["fact/src".to_owned(), SOURCE_SEAL.to_owned()]),
            writes: BTreeSet::from(["fact/out".to_owned()]),
            calls: Cell::new(0),
            seen: RefCell::new(Vec::new()),
        }
    }
}

impl FactGenerator for Gen {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn reads(&self) -> &BTreeSet<String> {
        &self.reads
    }

    fn writes(&self) -> &BTreeSet<String> {
        &self.writes
    }

    fn implementation(&self) -> ContentId {
        let facts = vec![Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "QualifiedOutputV1"),
            fields: vec![
                (Symbol::new("path"), Datum::String("fact/out".to_owned())),
                (Symbol::new("bytes"), Datum::Bytes(b"out".to_vec())),
            ],
        }];
        Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "GeneratorImplV1"),
            fields: vec![(Symbol::new("output"), Datum::List(facts))],
        }
        .content_id()
        .expect("implementation")
    }

    fn generate(&self, source: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        assert_eq!(
            source.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["fact/src", SOURCE_SEAL]
        );
        self.calls.set(self.calls.get() + 1);
        *self.seen.borrow_mut() = source["fact/src"].clone();
        BTreeMap::from([("fact/out".to_owned(), b"out".to_vec())])
    }
}

struct Flow {
    revision: super::Revision,
    facet: ArtifactFacet,
    generator: Gen,
}

fn flow() -> Flow {
    let seal = b"seal-v1".to_vec();
    let mut images = BTreeMap::new();
    images.insert(SOURCE_SEAL.to_owned(), seal.clone());
    images.insert("fact/src".to_owned(), b"src".to_vec());
    images.insert("fact/out".to_owned(), b"out".to_vec());
    let revision = open(OpenRequest {
        seal_id: seal_id(&seal).expect("seal"),
        seal,
        images,
        claims: vec![
            claim("claim/src", &["fact/src"]),
            claim("claim/seal", &[SOURCE_SEAL]),
        ],
    })
    .expect("revision");
    let owner = OwnerId::from_text("owner/out").expect("owner");
    Flow {
        facet: facet(&owner),
        generator: Gen::new(owner),
        revision,
    }
}

fn facet(owner: &OwnerId) -> ArtifactFacet {
    ArtifactFacet::new(
        ArtifactId::from_text("artifact/out").expect("artifact"),
        owner.clone(),
        RegionSelector::ExactPath("fact/out".to_owned()),
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        DisclosureDecisionId::from_text("disclosure/model").expect("disclosure"),
        RegionOwnership::Generated {
            generator: owner.clone(),
        },
    )
    .expect("facet")
}

fn votes(src: ProofOutcome) -> BTreeMap<String, ProofOutcome> {
    BTreeMap::from([
        ("claim/src".to_owned(), src),
        ("claim/seal".to_owned(), ProofOutcome::Passed),
    ])
}

fn seal(
    flow: &Flow,
    proofs: &BTreeMap<String, ProofOutcome>,
    prior: Option<&FixedSeal>,
) -> FixedStep {
    let generators: [&dyn FactGenerator; 1] = [&flow.generator];
    seal_fixed_point(
        &flow.revision,
        std::slice::from_ref(&flow.facet),
        &generators,
        proofs,
        &BTreeSet::new(),
        prior,
    )
    .expect("seal")
}

#[test]
fn change_proof_reclosure_seals_and_the_next_pass_does_no_work() {
    let mut flow = flow();
    let opened_id = flow.revision.id.clone();
    let opened_image = super::image_id(flow.revision.images()).expect("image");
    let seal_digest = flow.revision.claims["claim/seal"].digest.clone();
    let marked = {
        let step = super::propose(&mut flow.revision, malformed("opaque", 4, None)).expect("mark");
        assert!(matches!(step, Step::Malformed { .. }));
        flow.revision.malformed.clone()
    };

    let FixedStep::Sealed { work, .. } = seal(&flow, &votes(ProofOutcome::Passed), None) else {
        panic!("the closed revision seals");
    };
    assert_eq!(work, 1);
    assert_eq!(flow.generator.calls.get(), 1);
    assert_eq!(flow.generator.seen.borrow().as_slice(), b"src");

    let stale_bytes = closure_of(&flow.revision, "claim/src");
    let step =
        super::propose(&mut flow.revision, complete("fact/src", b"src-1", 100)).expect("propose");
    let Step::Invalidated {
        affected,
        closed,
        revision,
        ..
    } = step
    else {
        panic!("expected invalidation, got {step:?}");
    };
    assert_eq!(affected, BTreeSet::from(["claim/src".to_owned()]));
    assert!(closed.contains("claim/seal"));
    assert_eq!(revision, flow.revision.id);
    assert!(!flow.revision.claims["claim/src"].closed);
    assert!(flow.revision.claims["claim/src"].digest.is_none());
    assert_eq!(flow.revision.claims["claim/seal"].digest, seal_digest);
    let fenced = flow.revision.id.clone();
    let seen = flow.revision.seen_images.clone();
    assert!(seen.contains(&opened_image));

    let blocked = seal(&flow, &votes(ProofOutcome::Passed), None);
    assert_eq!(
        blocked,
        FixedStep::Blocked {
            reason: "open-claim",
            closure: BTreeSet::from(["claim/src".to_owned()]),
            repeated: None,
        }
    );
    assert_eq!(flow.generator.calls.get(), 1);
    assert!(!flow.revision.claims["claim/src"].closed);

    let current = closure_of(&flow.revision, "claim/src");
    assert_ne!(stale_bytes, current);
    let proofs = catalog([
        equality("leaf/stale", &stale_bytes),
        equality("leaf/now", &current),
    ]);
    let (journal, head) = open_journal();
    let stale = submit(
        &mut flow.revision,
        &proofs,
        &journal,
        &head,
        &[attachment("claim/src", "leaf/stale")],
    )
    .expect("stale");
    let super::AttachStep::Open {
        reasons,
        revision,
        head,
        ..
    } = stale
    else {
        panic!("stale bytes stay open, got {stale:?}");
    };
    assert_eq!(reasons.get("claim/src"), Some(&"stale-inputs"));
    assert_eq!(revision, fenced);
    assert!(!flow.revision.claims["claim/src"].closed);
    assert_eq!(flow.revision.id, fenced);

    let closed = submit(
        &mut flow.revision,
        &proofs,
        &journal,
        &head,
        &[attachment("claim/src", "leaf/now")],
    )
    .expect("close");
    let super::AttachStep::Closed {
        closed,
        open,
        reasons,
        revision,
        ..
    } = closed
    else {
        panic!("current closure closes, got {closed:?}");
    };
    assert_eq!(closed, BTreeSet::from(["claim/src".to_owned()]));
    assert!(open.is_empty());
    assert!(reasons.is_empty());
    assert_eq!(revision, flow.revision.id);
    assert_ne!(flow.revision.id, fenced);
    assert!(flow.revision.claims["claim/src"].closed);
    let digest = super::claim_digest(
        "claim/src",
        &flow.revision.claims["claim/src"].inputs,
        flow.revision.images(),
    )
    .expect("digest");
    assert_eq!(
        flow.revision.claims["claim/src"].digest.as_ref(),
        Some(&digest)
    );
    assert_eq!(flow.revision.claims["claim/seal"].digest, seal_digest);
    assert_eq!(flow.revision.images()["fact/src"], b"src-1");
    assert_eq!(flow.revision.images()["fact/out"], b"out");
    assert_eq!(flow.revision.seen_images, seen);
    assert_eq!(flow.revision.malformed, marked);
    assert!(!flow.revision.is_uncertain());
    assert!(flow.revision.seen_revisions.contains(&opened_id));
    assert!(flow.revision.seen_revisions.contains(&fenced));
    assert_eq!(
        recorded_proofs(&journal),
        vec![
            (proof_line(&fenced, "claim/src", "leaf/stale"), false),
            (proof_line(&fenced, "claim/src", "leaf/now"), true),
        ]
    );

    let failed = seal(&flow, &votes(ProofOutcome::Failed), None);
    assert_eq!(
        failed,
        FixedStep::Blocked {
            reason: "proof-failed",
            closure: BTreeSet::from(["claim/src".to_owned()]),
            repeated: None,
        }
    );
    assert!(flow.revision.claims["claim/src"].closed);
    assert_eq!(flow.generator.calls.get(), 1);

    let FixedStep::Sealed { seal: fresh, work } = seal(&flow, &votes(ProofOutcome::Passed), None)
    else {
        panic!("reclosed revision seals");
    };
    assert_eq!(work, 1);
    assert_eq!(flow.generator.calls.get(), 2);
    assert_eq!(flow.generator.seen.borrow().as_slice(), b"src-1");
    let FixedStep::ZeroWork { seal: again } =
        seal(&flow, &votes(ProofOutcome::Passed), Some(&fresh))
    else {
        panic!("matching seal does no work");
    };
    assert_eq!(again, fresh);
    assert_eq!(flow.generator.calls.get(), 2);

    let back = super::propose(&mut flow.revision, complete("fact/src", b"src", 100)).expect("back");
    let Step::Oscillation {
        repeated,
        delta,
        revision,
    } = back
    else {
        panic!("original image is still seen, got {back:?}");
    };
    assert_eq!(repeated, opened_image);
    assert_eq!(delta, BTreeSet::from(["fact/src".to_owned()]));
    assert_eq!(revision, flow.revision.id);
    assert_eq!(flow.revision.images()["fact/src"], b"src-1");
    assert!(flow.revision.claims["claim/src"].closed);
    assert_eq!(flow.revision.seen_images, seen);
    assert_eq!(flow.revision.malformed, marked);
    assert!(flow.revision.seen_revisions.contains(&opened_id));
}

#[test]
fn a_stale_journal_head_leaves_the_open_claim() {
    let mut revision = opened();
    reopen(&mut revision, "fact/alpha", b"alpha-1");
    let fenced = revision.id.clone();
    let current = closure_of(&revision, "claim/alpha");
    let leaves = catalog([equality("leaf/now", &current)]);
    let (journal, head) = open_journal();
    let moved = journal
        .append(
            Some(&head),
            ExecutionRecord::StateTransition {
                from: "planned".to_owned(),
                to: "running".to_owned(),
            },
            Vec::new(),
        )
        .expect("advance");
    let error = submit(
        &mut revision,
        &leaves,
        &journal,
        &head,
        &[attachment("claim/alpha", "leaf/now")],
    )
    .expect_err("stale head");
    assert!(matches!(
        error,
        super::AttachError::Journal(ExecutionJournalError::Journal(
            JournalError::WrongHead | JournalError::ConflictingDelivery
        ))
    ));
    assert_eq!(revision.id, fenced);
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(recorded_proofs(&journal).is_empty());
    assert_eq!(journal.rebuild().expect("rebuild").head, moved);
}

#[test]
fn a_rewritten_revision_id_is_not_the_fence() {
    let mut revision = opened();
    reopen(&mut revision, "fact/alpha", b"alpha-1");
    let accepted = revision.id.clone();
    let seal = revision.seal.clone();
    let current = closure_of(&revision, "claim/alpha");
    let leaves = catalog([equality("leaf/now", &current)]);
    let one = [attachment("claim/alpha", "leaf/now")];
    let forged = ContentId::from_bytes(Symbol::qualified("core", "sha256"), [7; 32]);
    revision.id = forged.clone();

    let (journal, head) = open_journal();
    let stale = super::attach_proofs(&mut revision, &leaves, &forged, &journal, &head, &one)
        .expect("forged id");
    match stale {
        super::AttachStep::Refused {
            revision: reported,
            reason,
        } => {
            assert_eq!(reason, "stale-revision");
            assert_eq!(reported, accepted);
        }
        other => panic!("forged id must not fence the call, got {other:?}"),
    }
    assert_eq!(revision.id, forged);
    assert_eq!(revision.seal, seal);
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(recorded_proofs(&journal).is_empty());

    let (journal, head) = open_journal();
    let step = super::attach_proofs(&mut revision, &leaves, &accepted, &journal, &head, &one)
        .expect("accepted fence");
    let super::AttachStep::Closed {
        revision: next,
        closed,
        ..
    } = step
    else {
        panic!("accepted fence closes the claim, got {step:?}");
    };
    assert_eq!(closed, BTreeSet::from(["claim/alpha".to_owned()]));
    assert_ne!(next, forged);
    assert_eq!(revision.id, next);
    assert_eq!(revision.seal, seal);
    assert!(revision.claims["claim/alpha"].closed);
    assert_eq!(
        recorded_proofs(&journal),
        vec![(proof_line(&accepted, "claim/alpha", "leaf/now"), true)]
    );

    let mut revision = opened();
    reopen(&mut revision, "fact/alpha", b"alpha-1");
    let accepted = revision.id.clone();
    let current = closure_of(&revision, "claim/alpha");
    let leaves = catalog([equality("leaf/now", &current)]);
    let one = [attachment("claim/alpha", "leaf/now")];
    revision.seal = forged.clone();
    revision.id = forged.clone();
    let (journal, head) = open_journal();
    let stale = super::attach_proofs(&mut revision, &leaves, &forged, &journal, &head, &one)
        .expect("forged seal");
    assert!(matches!(
        stale,
        super::AttachStep::Refused {
            reason: "stale-revision",
            ..
        }
    ));
    assert_eq!(revision.id, forged);
    assert_eq!(revision.seal, forged);
    assert!(!revision.claims["claim/alpha"].closed);
    assert!(recorded_proofs(&journal).is_empty());

    let (journal, head) = open_journal();
    let step = super::attach_proofs(&mut revision, &leaves, &accepted, &journal, &head, &one)
        .expect("image seal");
    assert!(matches!(step, super::AttachStep::Closed { .. }));
    assert_eq!(revision.seal, seal_id(b"seal-v1").expect("seal"));
    assert_ne!(revision.id, forged);
    assert!(revision.claims["claim/alpha"].closed);
    assert_eq!(
        recorded_proofs(&journal),
        vec![(proof_line(&accepted, "claim/alpha", "leaf/now"), true)]
    );
}
