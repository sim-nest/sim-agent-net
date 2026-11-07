// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use sim_roadmap_core::RoadmapSpec;
use sim_source_deck::SourceDeck;

use super::check::prepare;
use super::{AuthorError, CompiledAuthoring, Family, Selection};
use crate::{
    CanonicalWork, OpenClaim, POSTIMAGE_MODE, PortableImage, ProofCatalog, ProofLeaf, RevisionOpen,
    SealedMutationPlan, StructuralEdit, WorkSource, open_revision, seal_id,
};

const PHASE: &str = "authoring";

/// Compile the selected source into the existing revision, work, proof, and
/// mutation values.
///
/// The revision is opened and not committed. The work value is filled and not
/// received. The mutation plan is sealed and not applied. Consultation is a
/// list of content ids and is not a claim input.
pub fn compile_authoring(
    family: &Family,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<CompiledAuthoring, AuthorError> {
    let prepared = prepare(family, deck, spec)?;
    let selected = prepared.selected;
    let seal =
        seal_id(&selected.source).map_err(|error| AuthorError::Revision(error.to_string()))?;
    let revision = open_revision(RevisionOpen {
        seal: selected.source.clone(),
        seal_id: seal,
        images: BTreeMap::from([(selected.path.clone(), selected.source.clone())]),
        claims: selected
            .facts
            .iter()
            .map(|fact| OpenClaim {
                id: fact.clone(),
                inputs: BTreeSet::from([selected.path.clone()]),
            })
            .collect(),
    })
    .map_err(|error| AuthorError::Revision(error.to_string()))?;
    let work = CanonicalWork {
        namespace: "work".to_owned(),
        phase: PHASE.to_owned(),
        law: selected.work.clone(),
        owner_package: selected.owner.clone(),
        sources: vec![WorkSource {
            surface: selected.owner.clone(),
            path: selected.path.clone(),
            body: selected.source.clone(),
        }],
        source_role: "released-dependency".to_owned(),
        target_surface: selected.roadmap.clone(),
        target_role: "funded-target".to_owned(),
        target_status: "declared".to_owned(),
        budget_bytes: selected.source.len() as u64,
        budget_files: 1,
        budget_tokens: selected.source.len() as u64,
        budget_output: selected.source.len() as u64,
        resources: Vec::new(),
        copy: selected.ratchet.clone(),
        current: selected.ratchet.clone(),
        schema: "shape/proposal".to_owned(),
        mirrors_agree: false,
        max_attempts: 1,
        unresolved: 1,
        narrowed_to: 0,
    };
    let deck_id = content_hex(&deck.id().0);
    let proof = ProofLeaf::SourceDeckPredicate {
        name: "source-deck".to_owned(),
        actual_deck: deck_id.clone(),
        expected_deck: deck_id,
        required_claims: prepared.required,
        present_claims: selected.facts.iter().cloned().collect(),
    };
    ProofCatalog::new([proof.clone()]).map_err(|error| AuthorError::Proof(error.to_string()))?;
    let mutation = SealedMutationPlan::seal(vec![StructuralEdit {
        path: selected.path.clone(),
        preimage: PortableImage::absent(),
        postimage: PortableImage::file(selected.source.clone(), POSTIMAGE_MODE),
    }])
    .map_err(|error| AuthorError::Mutation(error.to_string()))?;
    Ok(CompiledAuthoring {
        source: selected.source.clone(),
        selection: Selection {
            name: selected.name.clone(),
            round: selected.round.clone(),
            source: selected.source.clone(),
        },
        revision,
        work,
        proof,
        mutation,
        consultation: prepared.consultation,
    })
}

fn content_hex(id: &sim_kernel::ContentId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(id.bytes.len() * 2);
    for byte in id.bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
