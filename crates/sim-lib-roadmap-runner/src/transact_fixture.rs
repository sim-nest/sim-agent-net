// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use sim_artifact_facet::{
    ArtifactFacet, ArtifactId, BaseImage, DisclosureDecisionId, IntendedImage, MergePolicy,
    OwnerId, PortableImage, ProjectionId, RegionOwnership, RegionSelector,
};

use super::{
    ArtifactModel, BoundDisclosure, DisclosureClass, FacetEdit, ModelBatch, ModelStore, ModelTip,
    RepoOutcome, RepoRequest, transact_facets,
};

pub(crate) const MODE: u32 = 0o644;

pub(crate) fn text(body: &str) -> PortableImage {
    PortableImage::file(body.as_bytes(), MODE).expect("image")
}

pub(crate) fn text_mode(body: &str, mode: u32) -> PortableImage {
    PortableImage::file(body.as_bytes(), mode).expect("image")
}

pub(crate) fn raw(body: &[u8]) -> PortableImage {
    PortableImage::file(body.to_vec(), MODE).expect("image")
}

pub(crate) fn owner(name: &str) -> OwnerId {
    OwnerId::from_text(name).expect("owner")
}

pub(crate) fn decision(name: &str) -> DisclosureDecisionId {
    DisclosureDecisionId::from_text(name).expect("disclosure")
}

pub(crate) fn public_disclosure() -> BoundDisclosure {
    BoundDisclosure {
        decision: decision("disclosure/model"),
        class: DisclosureClass::Public,
        public_sink: true,
    }
}

pub(crate) fn exact(path: &str, actor: &OwnerId, policy: MergePolicy) -> ArtifactFacet {
    spec(
        path,
        actor,
        RegionSelector::ExactPath(path.to_owned()),
        policy,
        RegionOwnership::Authored,
        "disclosure/model",
    )
}

pub(crate) fn lines(
    path: &str,
    actor: &OwnerId,
    start: u32,
    end: u32,
    policy: MergePolicy,
) -> ArtifactFacet {
    spec(
        path,
        actor,
        RegionSelector::LineRange {
            path: path.to_owned(),
            start,
            end,
        },
        policy,
        RegionOwnership::Authored,
        "disclosure/model",
    )
}

pub(crate) fn generated(path: &str, actor: &OwnerId, policy: MergePolicy) -> ArtifactFacet {
    spec(
        path,
        actor,
        RegionSelector::ExactPath(path.to_owned()),
        policy,
        RegionOwnership::Generated {
            generator: actor.clone(),
        },
        "disclosure/model",
    )
}

pub(crate) fn edit(spec: ArtifactFacet, base: PortableImage, intended: PortableImage) -> FacetEdit {
    let base = BaseImage::check(&spec, base).expect("base");
    let intended = IntendedImage::check(&spec, intended).expect("intended");
    FacetEdit {
        spec,
        base,
        intended,
    }
}

pub(crate) fn request(
    repository: &str,
    actor: &OwnerId,
    tip: &ModelTip,
    base_revision: &str,
    disclosure: BoundDisclosure,
    edits: Vec<FacetEdit>,
) -> RepoRequest {
    RepoRequest {
        repository: repository.to_owned(),
        actor: actor.clone(),
        expected_branch: tip.branch.clone(),
        expected_head: tip.head.clone(),
        base_revision: base_revision.to_owned(),
        disclosure,
        edits,
    }
}

pub(crate) fn tree(files: &[(&str, &str)]) -> BTreeMap<String, PortableImage> {
    files
        .iter()
        .map(|(path, body)| ((*path).to_owned(), text(body)))
        .collect()
}

pub(crate) fn seeded(name: &str, files: &[(&str, &str)]) -> (ModelStore, ModelTip) {
    let mut store = ModelStore::new();
    let tip = store
        .insert_repository(name, "main", tree(files))
        .expect("repository");
    (store, tip)
}

pub(crate) fn linewise() -> MergePolicy {
    MergePolicy::Linewise {
        max_lines: 32,
        max_cells: 4_096,
    }
}

pub(crate) fn advance(
    store: &mut ModelStore,
    repository: &str,
    tip: &ModelTip,
    actor: &OwnerId,
    path: &str,
    next: PortableImage,
) -> ModelTip {
    let current = store.image_at(repository, &tip.head, path);
    let spec = exact(path, actor, MergePolicy::Exact);
    let edit = edit(spec, current, next);
    let batch = transact_facets(
        store,
        &[request(
            repository,
            actor,
            tip,
            &tip.head,
            public_disclosure(),
            vec![edit],
        )],
    )
    .expect("advance");
    let super::RepoOutcome::Applied { tip: next_tip, .. } = &batch.outcomes()[0] else {
        panic!("advance did not commit: {batch:?}");
    };
    next_tip.clone()
}

fn spec(
    path: &str,
    actor: &OwnerId,
    region: RegionSelector,
    policy: MergePolicy,
    ownership: RegionOwnership,
    disclosure: &str,
) -> ArtifactFacet {
    ArtifactFacet::new(
        ArtifactId::from_text(&format!("artifact/{path}")).expect("artifact"),
        actor.clone(),
        region,
        ProjectionId::from_text("projection/file").expect("projection"),
        policy,
        decision(disclosure),
        ownership,
    )
    .expect("facet")
}

pub(crate) fn apply(
    store: &mut ModelStore,
    tip: &ModelTip,
    actor: &OwnerId,
    path: &str,
    next: &str,
) -> ModelTip {
    let batch = transact_facets(
        store,
        &[request(
            "alpha",
            actor,
            tip,
            &tip.head,
            public_disclosure(),
            vec![edit(
                exact(path, actor, MergePolicy::Exact),
                text("old\n"),
                text(next),
            )],
        )],
    )
    .expect("apply");
    applied_tip(&batch)
}

pub(crate) fn applied_tip(batch: &ModelBatch) -> ModelTip {
    let RepoOutcome::Applied { tip, .. } = &batch.outcomes()[0] else {
        panic!("expected a commit: {batch:?}");
    };
    tip.clone()
}

pub(crate) fn refusal(batch: &ModelBatch) -> &str {
    refusal_at(batch, 0)
}

pub(crate) fn refusal_at(batch: &ModelBatch, index: usize) -> &str {
    let RepoOutcome::Refused { reason, .. } = &batch.outcomes()[index] else {
        panic!("expected a refusal at {index}: {batch:?}");
    };
    assert!(!batch.settled());
    reason
}

pub(crate) fn refusal_paths(batch: &ModelBatch) -> Vec<&str> {
    let RepoOutcome::Refused { paths, .. } = &batch.outcomes()[0] else {
        panic!("expected a refusal: {batch:?}");
    };
    paths.iter().map(String::as_str).collect()
}
