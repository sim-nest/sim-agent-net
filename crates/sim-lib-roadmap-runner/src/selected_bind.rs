// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one externally supplied selected plan.
//!
//! The bytes and identities come from the caller. This module opens the
//! revision, seals the fixed point, and holds the mutation and landing stores.

use std::collections::{BTreeMap, BTreeSet};

#[path = "selected_mem.rs"]
mod mem;

use mem::{MemJournal, MemWorkspace};

use sha2::{Digest, Sha256};
use sim_artifact_facet::{
    ArtifactFacet, ArtifactId, BaseImage, DisclosureDecisionId, IntendedImage, MergePolicy,
    OwnerId, PortableImage as ArtifactImage, ProjectionId, RegionOwnership, RegionSelector,
};
use sim_kernel::{ContentId, Symbol};

use crate::{
    BoundDisclosure, DisclosureClass, FacetEdit, FactGenerator, FixedStep, GitModel, LandCache,
    LandWhy, ModelStore, ModelTip, MutationEngine, MutationError, MutationFence, MutationJournal,
    MutationWorkspace, OpenClaim, PortableImage, ProofCatalog, ProofLeaf, ProofOutcome,
    RepoRequest, Revision, RevisionOpen, Rollback, SOURCE_SEAL, SealedEntry, SealedMutationPlan,
    StructuralEdit, land_apply, land_recover, land_why, open_revision, seal_fixed_point, seal_id,
};

pub(super) struct Plan {
    pub provider: String,
    pub operation: String,
    pub execution: String,
    pub owner: String,
    pub generator: String,
    pub seal: Vec<u8>,
    pub authored_path: String,
    pub authored: Vec<u8>,
    pub generated_path: String,
    pub generated: Vec<u8>,
    pub claims: Vec<(String, String)>,
    pub proof_name: String,
    pub proof_left: Vec<u8>,
    pub proof_right: Vec<u8>,
    pub mutate_path: String,
    pub mutate_pre: Vec<u8>,
    pub mutate_post: Vec<u8>,
    pub land_path: String,
    pub land_base: Vec<u8>,
    pub land_next: Vec<u8>,
    pub direction: String,
    pub prove_document: Option<String>,
    pub world_document: Option<String>,
}

pub(super) struct Bound {
    pub revision: Revision,
    facets: Vec<ArtifactFacet>,
    copy: CopyOut,
    proofs: BTreeMap<String, ProofOutcome>,
    pub catalog: ProofCatalog,
    workspace: MemWorkspace,
    mutation_journal: MemJournal,
    pub mutation: SealedMutationPlan,
    pub store: ModelStore,
    pub cache: LandCache,
    pub prior_tip: ModelTip,
    pub land_actor: OwnerId,
    pub land_spec: ArtifactFacet,
    pub land_base: ArtifactImage,
    pub land_next: ArtifactImage,
    pub postimage_id: ContentId,
}

pub(super) fn parse(text: &str) -> Result<Plan, String> {
    let mut lines = text.lines().filter(|line| !line.is_empty());
    if lines.next() != Some("sim-selected-plan-v1") {
        return Err("selected plan is not an admitted document".into());
    }
    let mut fields: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for line in lines {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        let values: Vec<String> = parts.map(str::to_owned).collect();
        if values.is_empty() {
            return Err(format!("selected plan {key} has no value"));
        }
        fields.entry(key.to_owned()).or_default().push(values);
    }
    if let Some(key) = fields
        .keys()
        .find(|key| !KNOWN_LINES.contains(&key.as_str()))
    {
        return Err(format!("selected plan has unknown {key}"));
    }
    let claims = fields.get("claim").cloned().unwrap_or_default();
    if claims.is_empty() || claims.iter().any(|row| row.len() != 2) {
        return Err("selected plan lacks a claim".into());
    }
    let (authored_path, authored) = pair(&fields, "authored")?;
    let (generated_path, generated) = pair(&fields, "generated")?;
    let (proof_name, proof_left, proof_right) = triple(&fields, "proof")?;
    let (mutate_path, mutate_pre, mutate_post) = triple(&fields, "mutate")?;
    let (land_path, land_base, land_next) = triple(&fields, "land")?;
    Ok(Plan {
        provider: token(&fields, "provider")?,
        operation: token(&fields, "operation")?,
        execution: token(&fields, "execution")?,
        owner: token(&fields, "owner")?,
        generator: token(&fields, "generator")?,
        seal: token(&fields, "seal")?.into_bytes(),
        authored_path,
        authored: authored.into_bytes(),
        generated_path,
        generated: generated.into_bytes(),
        claims: claims
            .into_iter()
            .map(|row| (row[0].clone(), row[1].clone()))
            .collect(),
        proof_name,
        proof_left: proof_left.into_bytes(),
        proof_right: proof_right.into_bytes(),
        mutate_path,
        mutate_pre: mutate_pre.into_bytes(),
        mutate_post: mutate_post.into_bytes(),
        land_path,
        land_base: land_base.into_bytes(),
        land_next: land_next.into_bytes(),
        direction: token(&fields, "direction")?,
        prove_document: optional(&fields, "prove-document")?,
        world_document: optional(&fields, "world-document")?,
    })
}

const KNOWN_LINES: [&str; 15] = [
    "provider",
    "operation",
    "execution",
    "owner",
    "generator",
    "seal",
    "authored",
    "generated",
    "claim",
    "proof",
    "mutate",
    "land",
    "direction",
    "prove-document",
    "world-document",
];

fn row(
    fields: &BTreeMap<String, Vec<Vec<String>>>,
    key: &str,
    len: usize,
) -> Result<Vec<String>, String> {
    let rows = fields
        .get(key)
        .ok_or_else(|| format!("selected plan lacks {key}"))?;
    if rows.len() != 1 || rows[0].len() != len {
        return Err(format!("selected plan {key} has the wrong shape"));
    }
    Ok(rows[0].clone())
}

fn token(fields: &BTreeMap<String, Vec<Vec<String>>>, key: &str) -> Result<String, String> {
    Ok(row(fields, key, 1)?.remove(0))
}

fn pair(
    fields: &BTreeMap<String, Vec<Vec<String>>>,
    key: &str,
) -> Result<(String, String), String> {
    let mut row = row(fields, key, 2)?;
    let second = row.remove(1);
    Ok((row.remove(0), second))
}

fn triple(
    fields: &BTreeMap<String, Vec<Vec<String>>>,
    key: &str,
) -> Result<(String, String, String), String> {
    let mut row = row(fields, key, 3)?;
    let third = row.remove(2);
    let second = row.remove(1);
    Ok((row.remove(0), second, third))
}

fn optional(
    fields: &BTreeMap<String, Vec<Vec<String>>>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(rows) = fields.get(key) else {
        return Ok(None);
    };
    if rows.len() != 1 || rows[0].len() != 1 {
        return Err(format!("selected plan {key} has the wrong shape"));
    }
    Ok(Some(rows[0][0].clone()))
}

pub(super) fn bind(plan: &Plan) -> Result<Bound, String> {
    let revision = open_plan(plan)?;
    let owner = OwnerId::from_text(&plan.owner).map_err(|err| err.to_string())?;
    let generator = OwnerId::from_text(&plan.generator).map_err(|err| err.to_string())?;
    let facets = vec![
        facet(&plan.authored_path, &owner, RegionOwnership::Authored)?,
        facet(
            &plan.generated_path,
            &generator,
            RegionOwnership::Generated {
                generator: generator.clone(),
            },
        )?,
    ];
    let copy = CopyOut::new(
        generator,
        &plan.authored_path,
        &plan.generated_path,
        &plan.generated,
    );
    let mut proofs = BTreeMap::new();
    for (id, _) in &plan.claims {
        proofs.insert(id.clone(), ProofOutcome::Passed);
    }
    let catalog = ProofCatalog::new([ProofLeaf::ArtifactEquality {
        name: plan.proof_name.clone(),
        left: plan.proof_left.clone(),
        right: plan.proof_right.clone(),
    }])
    .map_err(|err| err.to_string())?;
    let mutation = SealedMutationPlan::seal(vec![StructuralEdit {
        path: plan.mutate_path.clone(),
        preimage: PortableImage::file(plan.mutate_pre.clone(), 0o644),
        postimage: PortableImage::file(plan.mutate_post.clone(), 0o644),
    }])
    .map_err(|err| err.to_string())?;
    let workspace = MemWorkspace::with(&plan.mutate_path, &plan.mutate_pre);
    let land_actor = owner;
    let land_spec = facet(&plan.land_path, &land_actor, RegionOwnership::Authored)?;
    let land_base = artifact_file(&plan.land_base)?;
    let land_next = artifact_file(&plan.land_next)?;
    let mut store = ModelStore::new();
    let prior_tip = store
        .insert_repository(
            "selected",
            "main",
            BTreeMap::from([(plan.land_path.clone(), land_base.clone())]),
        )
        .map_err(|err| err.to_string())?;
    Ok(Bound {
        revision,
        facets,
        copy,
        proofs,
        catalog,
        workspace,
        mutation_journal: MemJournal::default(),
        mutation,
        store,
        cache: LandCache::new(),
        prior_tip,
        land_actor,
        land_spec,
        land_base,
        land_next,
        postimage_id: content(&plan.land_next),
    })
}

pub(super) fn seal(bound: &Bound) -> Result<String, String> {
    let generators: [&dyn FactGenerator; 1] = [&bound.copy];
    let sealed = seal_fixed_point(
        &bound.revision,
        &bound.facets,
        &generators,
        &bound.proofs,
        &BTreeSet::new(),
        None,
    )
    .map_err(|err| err.to_string())?;
    let FixedStep::Sealed { seal, work } = sealed else {
        return Err(format!("fixed point refused: {sealed:?}"));
    };
    if work != 1 {
        return Err(format!("fixed point ran {work} generators"));
    }
    Ok(hex(&seal.id))
}

pub(super) fn mutate(bound: &mut Bound) -> Result<(), String> {
    let files = std::mem::take(&mut bound.workspace.files);
    let journal = std::mem::take(&mut bound.mutation_journal);
    let plan = bound.mutation.clone();
    let mut engine = MutationEngine {
        workspace: MemWorkspace { files },
        journal,
    };
    let executed = engine.execute(&plan).map_err(|err| err.to_string());
    bound.workspace = engine.workspace;
    bound.mutation_journal = engine.journal;
    executed?;
    let observed = bound
        .workspace
        .observe(&plan.entries[0].path)
        .map_err(|err| err.to_string())?;
    if observed != plan.entries[0].postimage {
        return Err("mutation did not apply the postimage".into());
    }
    Ok(())
}

pub(super) fn land(bound: &mut Bound) -> Result<ModelTip, String> {
    let tip = bound
        .store
        .tip("selected")
        .ok_or_else(|| "landing repository is missing".to_owned())?;
    let edit = FacetEdit {
        spec: bound.land_spec.clone(),
        base: BaseImage::check(&bound.land_spec, bound.land_base.clone())
            .map_err(|err| err.to_string())?,
        intended: IntendedImage::check(&bound.land_spec, bound.land_next.clone())
            .map_err(|err| err.to_string())?,
    };
    let request = RepoRequest {
        repository: "selected".into(),
        actor: bound.land_actor.clone(),
        expected_branch: tip.branch.clone(),
        expected_head: tip.head.clone(),
        base_revision: tip.head.clone(),
        disclosure: BoundDisclosure {
            decision: DisclosureDecisionId::from_text("disclosure/model")
                .map_err(|err| err.to_string())?,
            class: DisclosureClass::Public,
            public_sink: false,
        },
        edits: vec![edit],
    };
    let batch = land_apply(&mut bound.store, &mut bound.cache, &[request])
        .map_err(|err| err.to_string())?;
    let crate::RepoOutcome::Applied { tip: next, .. } = &batch.outcomes()[0] else {
        return Err(format!("landing did not commit: {batch:?}"));
    };
    Ok(next.clone())
}

pub(super) fn delivery_current(bound: &Bound) -> Result<String, String> {
    match land_why(&bound.store, &bound.cache, "selected") {
        LandWhy::Current { head, .. } => Ok(head),
        other => Err(format!("delivery is not current: {other:?}")),
    }
}

pub(super) fn recover(bound: &mut Bound, current: &ModelTip) -> Result<(), String> {
    let rolled = land_recover(
        &mut bound.store,
        &mut bound.cache,
        "selected",
        current,
        &bound.prior_tip.head,
    )
    .map_err(|err| err.to_string())?;
    let Rollback::Restored { .. } = rolled else {
        return Err(format!("recovery did not restore: {rolled:?}"));
    };
    Ok(())
}

fn open_plan(plan: &Plan) -> Result<Revision, String> {
    let mut images = BTreeMap::new();
    images.insert(SOURCE_SEAL.to_owned(), plan.seal.clone());
    images.insert(plan.authored_path.clone(), plan.authored.clone());
    images.insert(plan.generated_path.clone(), plan.generated.clone());
    let claims = plan
        .claims
        .iter()
        .map(|(id, path)| OpenClaim {
            id: id.clone(),
            inputs: BTreeSet::from([path.clone()]),
        })
        .collect();
    open_revision(RevisionOpen {
        seal: plan.seal.clone(),
        seal_id: seal_id(&plan.seal).map_err(|err| err.to_string())?,
        images,
        claims,
    })
    .map_err(|err| err.to_string())
}

fn facet(path: &str, owner: &OwnerId, ownership: RegionOwnership) -> Result<ArtifactFacet, String> {
    ArtifactFacet::new(
        ArtifactId::from_text(&format!("artifact/{path}")).map_err(|err| err.to_string())?,
        owner.clone(),
        RegionSelector::ExactPath(path.to_owned()),
        ProjectionId::from_text("projection/file").map_err(|err| err.to_string())?,
        MergePolicy::Exact,
        DisclosureDecisionId::from_text("disclosure/model").map_err(|err| err.to_string())?,
        ownership,
    )
    .map_err(|err| err.to_string())
}

fn artifact_file(bytes: &[u8]) -> Result<ArtifactImage, String> {
    ArtifactImage::file(bytes.to_vec(), 0o644).map_err(|err| err.to_string())
}

pub(super) fn content(bytes: &[u8]) -> ContentId {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), digest)
}

pub(super) fn cid_byte(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
}

pub(super) fn hex(id: &ContentId) -> String {
    id.bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct CopyOut {
    owner: OwnerId,
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    path: String,
    output: Vec<u8>,
    implementation: ContentId,
}

impl CopyOut {
    fn new(owner: OwnerId, read: &str, path: &str, output: &[u8]) -> Self {
        Self {
            owner,
            reads: BTreeSet::from([read.to_owned(), SOURCE_SEAL.to_owned()]),
            writes: BTreeSet::from([path.to_owned()]),
            path: path.to_owned(),
            output: output.to_vec(),
            implementation: content(output),
        }
    }
}

impl FactGenerator for CopyOut {
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
        self.implementation.clone()
    }
    fn generate(&self, source: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        if !source.contains_key(SOURCE_SEAL) {
            return BTreeMap::new();
        }
        BTreeMap::from([(self.path.clone(), self.output.clone())])
    }
}
