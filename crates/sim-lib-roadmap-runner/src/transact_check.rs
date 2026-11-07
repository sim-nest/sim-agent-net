// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Refusals that run before any model commit.

use std::collections::BTreeSet;

use sim_artifact_facet::{ArtifactFacet, RegionOwnership, RegionSelector};

use super::{
    ArtifactModel, BoundDisclosure, DisclosureClass, FacetEdit, GitModel, ModelTip, RepoRequest,
};

pub(super) struct Refusal {
    pub reason: &'static str,
    pub paths: BTreeSet<String>,
}

pub(super) fn precheck<S>(store: &S, request: &RepoRequest, tip: &ModelTip) -> Option<Refusal>
where
    S: ArtifactModel + GitModel,
{
    if tip.branch != request.expected_branch || tip.head != request.expected_head {
        return Some(refuse("wrong-head", [&request.repository]));
    }
    let dirt = store.dirt(&request.repository);
    if !dirt.is_empty() {
        return Some(Refusal {
            reason: "dirt",
            paths: dirt,
        });
    }
    if let Some(paths) = private_flow(request) {
        return Some(Refusal {
            reason: "private-flow",
            paths,
        });
    }
    if let Some(refusal) = owner_grant(request) {
        return Some(refusal);
    }
    stale_base(store, request)
}

pub(super) fn region_path(spec: &ArtifactFacet) -> &str {
    match spec.region() {
        RegionSelector::ExactPath(path) | RegionSelector::LineRange { path, .. } => path,
    }
}

fn private_flow(request: &RepoRequest) -> Option<BTreeSet<String>> {
    let blocked = class_blocks(&request.disclosure);
    let mut paths = BTreeSet::new();
    for edit in &request.edits {
        if blocked || edit.spec.disclosure() != &request.disclosure.decision {
            paths.insert(region_path(&edit.spec).to_owned());
        }
    }
    if blocked && paths.is_empty() {
        paths.insert(request.repository.clone());
    }
    (!paths.is_empty()).then_some(paths)
}

fn class_blocks(disclosure: &BoundDisclosure) -> bool {
    match disclosure.class {
        DisclosureClass::Unknown => disclosure.public_sink,
        DisclosureClass::Private => disclosure.public_sink,
        DisclosureClass::Public => false,
    }
}

fn owner_grant(request: &RepoRequest) -> Option<Refusal> {
    let mut bypass = BTreeSet::new();
    let mut owners = BTreeSet::new();
    for edit in &request.edits {
        let path = region_path(&edit.spec);
        match edit.spec.ownership() {
            RegionOwnership::Generated { generator } if generator != &request.actor => {
                bypass.insert(path.to_owned());
            }
            RegionOwnership::Generated { .. } | RegionOwnership::Authored => {
                if edit.spec.owner() != &request.actor {
                    owners.insert(path.to_owned());
                }
            }
        }
    }
    if !bypass.is_empty() {
        return Some(Refusal {
            reason: "generated-owner-bypass",
            paths: bypass,
        });
    }
    if !owners.is_empty() {
        return Some(Refusal {
            reason: "wrong-owner",
            paths: owners,
        });
    }
    None
}

fn stale_base<S: ArtifactModel>(store: &S, request: &RepoRequest) -> Option<Refusal> {
    if !store.revision_exists(&request.repository, &request.base_revision) {
        let paths = edit_paths(&request.edits);
        let paths = if paths.is_empty() {
            BTreeSet::from([request.repository.clone()])
        } else {
            paths
        };
        return Some(Refusal {
            reason: "stale-base",
            paths,
        });
    }
    let mut paths = BTreeSet::new();
    for edit in &request.edits {
        let path = region_path(&edit.spec);
        let historical = store.image_at(&request.repository, &request.base_revision, path);
        if &historical != edit.base.image() {
            paths.insert(path.to_owned());
        }
    }
    (!paths.is_empty()).then_some(Refusal {
        reason: "stale-base",
        paths,
    })
}

fn edit_paths(edits: &[FacetEdit]) -> BTreeSet<String> {
    edits
        .iter()
        .map(|edit| region_path(&edit.spec).to_owned())
        .collect()
}

fn refuse(reason: &'static str, paths: impl IntoIterator<Item = impl AsRef<str>>) -> Refusal {
    Refusal {
        reason,
        paths: paths
            .into_iter()
            .map(|path| path.as_ref().to_owned())
            .collect(),
    }
}
