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

use super::{FactGenerator, FixedError, FixedSeal, FixedStep, seal_fixed_point};
use crate::{OpenClaim, ProofOutcome, Revision, RevisionOpen, SOURCE_SEAL, open_revision, seal_id};

pub(super) struct Gen {
    pub owner: OwnerId,
    pub reads: BTreeSet<String>,
    pub writes: BTreeSet<String>,
    pub output: RefCell<BTreeMap<String, Vec<u8>>>,
    pub calls: Cell<u32>,
    pub seen: RefCell<Option<BTreeMap<String, Vec<u8>>>>,
}

impl Gen {
    pub(super) fn new(owner: &OwnerId, reads: &[&str], writes: &[(&str, &[u8])]) -> Self {
        let mut output = BTreeMap::new();
        let mut write_set = BTreeSet::new();
        for (path, bytes) in writes {
            write_set.insert((*path).to_owned());
            output.insert((*path).to_owned(), (*bytes).to_vec());
        }
        Self {
            owner: owner.clone(),
            reads: names(reads),
            writes: write_set,
            output: RefCell::new(output),
            calls: Cell::new(0),
            seen: RefCell::new(None),
        }
    }

    pub(super) fn twin(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            reads: self.reads.clone(),
            writes: self.writes.clone(),
            output: RefCell::new(self.output.borrow().clone()),
            calls: Cell::new(0),
            seen: RefCell::new(None),
        }
    }

    pub(super) fn set(&self, path: &str, bytes: &[u8]) {
        self.output
            .borrow_mut()
            .insert(path.to_owned(), bytes.to_vec());
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
        let facts = self
            .output
            .borrow()
            .iter()
            .map(|(path, bytes)| Datum::Node {
                tag: Symbol::qualified("roadmap-runner", "QualifiedOutputV1"),
                fields: vec![
                    (Symbol::new("path"), Datum::String(path.clone())),
                    (Symbol::new("bytes"), Datum::Bytes(bytes.clone())),
                ],
            })
            .collect();
        Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "GeneratorImplV1"),
            fields: vec![(Symbol::new("output"), Datum::List(facts))],
        }
        .content_id()
        .expect("qualified implementation")
    }

    fn generate(&self, source: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        *self.seen.borrow_mut() = Some(source.clone());
        self.output.borrow().clone()
    }
}

pub(super) struct World {
    pub revision: Revision,
    pub facets: Vec<ArtifactFacet>,
    pub alpha: Gen,
    pub beta: Gen,
    pub proofs: BTreeMap<String, ProofOutcome>,
}

pub(super) fn world() -> World {
    revision_world(true)
}

/// Same files as [`world`], without a claim that reads `fact/alpha`.
pub(super) fn world_alpha_unclaimed() -> World {
    revision_world(false)
}

fn revision_world(claim_alpha: bool) -> World {
    let seal = b"seal-v1".to_vec();
    let mut images = BTreeMap::new();
    images.insert(SOURCE_SEAL.to_owned(), seal.clone());
    images.insert("fact/src".to_owned(), b"src".to_vec());
    images.insert("fact/alpha".to_owned(), b"alpha".to_vec());
    images.insert("fact/extra".to_owned(), b"extra".to_vec());
    images.insert("fact/beta".to_owned(), b"beta".to_vec());
    let revision = open_revision(RevisionOpen {
        seal_id: seal_id(&seal).expect("seal"),
        seal,
        images,
        claims: {
            let mut claims = vec![
                claim("claim/extra", &["fact/extra"]),
                claim("claim/beta", &["fact/beta"]),
                claim("claim/src", &["fact/src"]),
            ];
            if claim_alpha {
                claims.insert(0, claim("claim/alpha", &["fact/alpha"]));
            }
            claims
        },
    })
    .expect("revision");
    let alpha_owner = OwnerId::from_text("owner/alpha").expect("owner");
    let beta_owner = OwnerId::from_text("owner/beta").expect("owner");
    let src_owner = OwnerId::from_text("owner/src").expect("owner");
    let facets = vec![
        facet("artifact/src", &src_owner, "fact/src", false),
        facet("artifact/alpha", &alpha_owner, "fact/alpha", true),
        facet("artifact/alpha", &alpha_owner, "fact/extra", true),
        facet("artifact/beta", &beta_owner, "fact/beta", true),
    ];
    let alpha = Gen::new(
        &alpha_owner,
        &["fact/src", SOURCE_SEAL],
        &[("fact/alpha", b"alpha"), ("fact/extra", b"extra")],
    );
    let beta = Gen::new(&beta_owner, &["fact/src"], &[("fact/beta", b"beta")]);
    let mut proofs = BTreeMap::new();
    let mut ids = vec!["claim/extra", "claim/beta", "claim/src"];
    if claim_alpha {
        ids.insert(0, "claim/alpha");
    }
    for id in ids {
        proofs.insert(id.to_owned(), ProofOutcome::Passed);
    }
    World {
        revision,
        facets,
        alpha,
        beta,
        proofs,
    }
}

pub(super) fn facet(artifact: &str, owner: &OwnerId, path: &str, generated: bool) -> ArtifactFacet {
    let ownership = if generated {
        RegionOwnership::Generated {
            generator: owner.clone(),
        }
    } else {
        RegionOwnership::Authored
    };
    ArtifactFacet::new(
        ArtifactId::from_text(artifact).expect("artifact"),
        owner.clone(),
        RegionSelector::ExactPath(path.to_owned()),
        ProjectionId::from_text("projection/file").expect("projection"),
        MergePolicy::Exact,
        DisclosureDecisionId::from_text("disclosure/model").expect("disclosure"),
        ownership,
    )
    .expect("facet")
}

pub(super) fn seal(
    world: &World,
    deleted: &BTreeSet<String>,
    prior: Option<&FixedSeal>,
) -> Result<FixedStep, FixedError> {
    let generators: [&dyn FactGenerator; 2] = [&world.alpha, &world.beta];
    seal_fixed_point(
        &world.revision,
        &world.facets,
        &generators,
        &world.proofs,
        deleted,
        prior,
    )
}

pub(super) fn calls(world: &World) -> u32 {
    world.alpha.calls.get() + world.beta.calls.get()
}

pub(super) fn paths(items: &[&str]) -> BTreeSet<String> {
    names(items)
}

pub(super) fn refusal(reason: &'static str, closure: &[&str]) -> FixedStep {
    FixedStep::Blocked {
        reason,
        closure: paths(closure),
        repeated: None,
    }
}

fn claim(id: &str, inputs: &[&str]) -> OpenClaim {
    OpenClaim {
        id: id.to_owned(),
        inputs: names(inputs),
    }
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}
