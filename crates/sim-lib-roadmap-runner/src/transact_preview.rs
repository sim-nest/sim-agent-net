// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dry observation of one repository. The branch pointer stays put.

use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::PortableImage;

use super::{
    ArtifactModel, CleanKind, GitModel, ModelTip, PreparedOrRefused, RepoRequest, TransactError,
    prepare,
};

/// What one repository would do. Nothing has been committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FacetPreview {
    /// A lawful merge would record one commit.
    WouldCommit {
        /// Repository name.
        repository: String,
        /// Pointer that would stay until apply.
        tip: ModelTip,
        /// Tree the commit would store.
        files: BTreeMap<String, PortableImage>,
    },
    /// The law asks for no write.
    Clean {
        /// Repository name.
        repository: String,
        /// Pointer that stays.
        tip: ModelTip,
        /// Why no commit is needed.
        kind: CleanKind,
    },
    /// The repository would be left untouched.
    Refused {
        /// Repository name.
        repository: String,
        /// Stable reason.
        reason: &'static str,
        /// Paths or the repository named by the refusal.
        paths: BTreeSet<String>,
    },
}

pub(crate) fn preview_repository<S>(
    store: &S,
    request: &RepoRequest,
) -> Result<FacetPreview, TransactError>
where
    S: ArtifactModel + GitModel,
{
    Ok(match prepare(store, request)? {
        PreparedOrRefused::Refused { reason, paths } => FacetPreview::Refused {
            repository: request.repository.clone(),
            reason,
            paths,
        },
        PreparedOrRefused::Ready(prepared) if !prepared.changed => FacetPreview::Clean {
            repository: request.repository.clone(),
            tip: prepared.tip,
            kind: prepared.kind,
        },
        PreparedOrRefused::Ready(prepared) => FacetPreview::WouldCommit {
            repository: request.repository.clone(),
            tip: prepared.tip,
            files: prepared.files,
        },
    })
}
