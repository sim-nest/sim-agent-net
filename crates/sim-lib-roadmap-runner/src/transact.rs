// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Model facet transactions over the shared three-way merge law.
//!
//! Each repository is its own commit. A refusal leaves that repository's
//! branch where it was. The call does not open a checkout or move a host Git
//! ref. Symlink races and physical file replacement stay outside this model.

use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::{
    ArtifactFacet, BaseImage, ConflictReason, DisclosureDecisionId, FacetError, IntendedImage,
    MergeOutcome, ObservedImage, OwnerId, PortableImage, merge_facet,
};
use thiserror::Error;

#[path = "transact_check.rs"]
mod check;
#[path = "transact_preview.rs"]
mod preview;
#[path = "transact_store.rs"]
mod store;

pub(crate) use preview::{FacetPreview, preview_repository};

pub use store::{ArtifactModel, GitFault, GitModel, ModelStore, ModelTip};

use check::region_path;
use store::GitFault as Fault;

/// External disclosure decision. It is not a label the facet invents for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisclosureClass {
    /// The decision admits a public sink.
    Public,
    /// The decision stays off a public sink.
    Private,
    /// The decision is missing or disagrees with its rule set.
    Unknown,
}

/// Disclosure bound to one repository transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundDisclosure {
    /// Decision id the caller supplies from the audit rule set.
    pub decision: DisclosureDecisionId,
    /// Class of that decision.
    pub class: DisclosureClass,
    /// Whether this transaction would reach a public sink.
    pub public_sink: bool,
}

/// One role-checked edit inside a repository transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetEdit {
    /// Owner, region, merge policy, and disclosure id.
    pub spec: ArtifactFacet,
    /// Image sealed at `base_revision`.
    pub base: BaseImage,
    /// Image the caller wants the path to become.
    pub intended: IntendedImage,
}

/// One repository's model transaction. It does not name another repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRequest {
    /// Repository the model provider holds.
    pub repository: String,
    /// Owner allowed to write every edit in this transaction.
    pub actor: OwnerId,
    /// Branch the caller expects the pointer to name.
    pub expected_branch: String,
    /// Commit the caller expects the pointer to name.
    pub expected_head: String,
    /// Revision the base images were sealed against.
    pub base_revision: String,
    /// Disclosure decision for the whole transaction.
    pub disclosure: BoundDisclosure,
    /// Edits compared against one observation of this repository.
    pub edits: Vec<FacetEdit>,
}

/// Why a model transaction could not form an identity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransactError {
    /// A content id could not be formed.
    #[error("identity: {0}")]
    Identity(String),
    /// The facet law rejected an image before a refusal class applied.
    #[error("facet: {0}")]
    Facet(String),
}

/// A repository that needed no new commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanKind {
    /// The observed tree already holds the intended bytes.
    AlreadyTrue,
    /// The proposal matches its base, so the observed tree stays.
    Unchanged,
}

/// Result of one repository transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoOutcome {
    /// Every edit landed in one new commit.
    Applied {
        /// Repository name.
        repository: String,
        /// Pointer before the commit.
        previous: ModelTip,
        /// Pointer after the commit.
        tip: ModelTip,
    },
    /// The law asked for no write.
    Clean {
        /// Repository name.
        repository: String,
        /// Pointer that stayed in place.
        tip: ModelTip,
        /// Why no commit was recorded.
        kind: CleanKind,
    },
    /// The repository was left untouched.
    Refused {
        /// Repository name.
        repository: String,
        /// Stable reason.
        reason: &'static str,
        /// Paths or the repository named by the refusal.
        paths: BTreeSet<String>,
    },
}

/// Independent outcomes for one batch of repository transactions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelBatch {
    /// Every repository applied or needed no write.
    Settled {
        /// One outcome per request, in request order.
        outcomes: Vec<RepoOutcome>,
    },
    /// At least one repository refused. Repositories that committed stay committed.
    Recovered {
        /// One outcome per request, in request order.
        outcomes: Vec<RepoOutcome>,
    },
}

impl ModelBatch {
    /// Reports whether every repository completed without a refusal.
    pub const fn settled(&self) -> bool {
        matches!(self, Self::Settled { .. })
    }

    /// Returns the per-repository outcomes.
    pub fn outcomes(&self) -> &[RepoOutcome] {
        match self {
            Self::Settled { outcomes } | Self::Recovered { outcomes } => outcomes,
        }
    }
}

/// Result of moving a model branch pointer back to an existing commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rollback {
    /// The pointer now names `tip`. The target commit's bytes are unchanged.
    Restored {
        /// Pointer after the move.
        tip: ModelTip,
    },
    /// The pointer did not move.
    Refused {
        /// Stable reason.
        reason: &'static str,
        /// Paths or the repository named by the refusal.
        paths: BTreeSet<String>,
    },
}

/// Compares each request through [`merge_facet`] and commits a repository only
/// when every edit in that request is lawful.
///
/// A later request sees commits made by an earlier request. A refusal does not
/// rewind those commits. An identity or facet error returns after any earlier
/// repository commit has already landed.
pub fn transact_facets<S>(
    store: &mut S,
    requests: &[RepoRequest],
) -> Result<ModelBatch, TransactError>
where
    S: ArtifactModel + GitModel,
{
    let mut outcomes = Vec::with_capacity(requests.len());
    let mut settled = true;
    for request in requests {
        let outcome = one_repository(store, request)?;
        if matches!(outcome, RepoOutcome::Refused { .. }) {
            settled = false;
        }
        outcomes.push(outcome);
    }
    Ok(if settled {
        ModelBatch::Settled { outcomes }
    } else {
        ModelBatch::Recovered { outcomes }
    })
}

/// Moves one model branch pointer to an existing commit.
///
/// Rollback does not compare facets and it does not rewrite the commit it
/// leaves behind.
pub fn rollback_model<S>(
    store: &mut S,
    repository: &str,
    expected: &ModelTip,
    target: &str,
) -> Result<Rollback, TransactError>
where
    S: GitModel,
{
    match store.rollback(repository, expected, target) {
        Ok(tip) => Ok(Rollback::Restored { tip }),
        Err(Fault::WrongHead) => Ok(refused_rollback("wrong-head", repository)),
        Err(Fault::Dirt) => Ok(Rollback::Refused {
            reason: "dirt",
            paths: store.dirt(repository),
        }),
        Err(Fault::Identity(text)) => Err(TransactError::Identity(text)),
    }
}

struct Prepared {
    tip: ModelTip,
    files: BTreeMap<String, PortableImage>,
    changed: bool,
    kind: CleanKind,
}

enum PreparedOrRefused {
    Ready(Prepared),
    Refused {
        reason: &'static str,
        paths: BTreeSet<String>,
    },
}

fn prepare<S>(store: &S, request: &RepoRequest) -> Result<PreparedOrRefused, TransactError>
where
    S: ArtifactModel + GitModel,
{
    let Some(tip) = store.tip(&request.repository) else {
        return Ok(refused_prep("wrong-head", [&request.repository]));
    };
    if let Some(refusal) = check::precheck(store, request, &tip) {
        return Ok(PreparedOrRefused::Refused {
            reason: refusal.reason,
            paths: refusal.paths,
        });
    }
    match judge(store, request, &tip)? {
        Judged::Refuse(refusal) => Ok(PreparedOrRefused::Refused {
            reason: refusal.reason,
            paths: refusal.paths,
        }),
        Judged::Ready {
            files,
            changed,
            kind,
        } => Ok(PreparedOrRefused::Ready(Prepared {
            tip,
            files,
            changed,
            kind,
        })),
    }
}

fn one_repository<S>(store: &mut S, request: &RepoRequest) -> Result<RepoOutcome, TransactError>
where
    S: ArtifactModel + GitModel,
{
    match prepare(store, request)? {
        PreparedOrRefused::Refused { reason, paths } => Ok(RepoOutcome::Refused {
            repository: request.repository.clone(),
            reason,
            paths,
        }),
        PreparedOrRefused::Ready(Prepared {
            changed: false,
            tip,
            kind,
            ..
        }) => Ok(RepoOutcome::Clean {
            repository: request.repository.clone(),
            tip,
            kind,
        }),
        PreparedOrRefused::Ready(Prepared {
            changed: true,
            files,
            tip,
            ..
        }) => match store.commit(&request.repository, &tip, &files) {
            Ok(new_tip) => Ok(RepoOutcome::Applied {
                repository: request.repository.clone(),
                previous: tip,
                tip: new_tip,
            }),
            Err(Fault::WrongHead) => Ok(refused(request, "wrong-head", [&request.repository])),
            Err(Fault::Dirt) => Ok(RepoOutcome::Refused {
                repository: request.repository.clone(),
                reason: "dirt",
                paths: store.dirt(&request.repository),
            }),
            Err(Fault::Identity(text)) => Err(TransactError::Identity(text)),
        },
    }
}

fn refused_prep(
    reason: &'static str,
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> PreparedOrRefused {
    PreparedOrRefused::Refused {
        reason,
        paths: paths
            .into_iter()
            .map(|path| path.as_ref().to_owned())
            .collect(),
    }
}

enum Judged {
    Ready {
        files: BTreeMap<String, PortableImage>,
        changed: bool,
        kind: CleanKind,
    },
    Refuse(check::Refusal),
}

fn judge<S>(store: &S, request: &RepoRequest, tip: &ModelTip) -> Result<Judged, TransactError>
where
    S: ArtifactModel,
{
    let Some(tree) = store.tree(&request.repository, &tip.head) else {
        return Ok(Judged::Refuse(one_path("wrong-head", &request.repository)));
    };
    let mut files = tree;
    let mut changed = false;
    let mut saw_edit = false;
    let mut every_already = true;
    for (path, edits) in group_edits(&request.edits) {
        let original = files
            .get(&path)
            .cloned()
            .unwrap_or_else(PortableImage::absent);
        let mut current = original.clone();
        for edit in edits {
            saw_edit = true;
            let observed = match ObservedImage::check(&edit.spec, current.clone()) {
                Ok(image) => image,
                Err(error) => return Err(facet_error(error)),
            };
            let outcome = match merge_facet(&edit.spec, &edit.base, &observed, &edit.intended) {
                Ok(outcome) => outcome,
                Err(FacetError::OutOfRegion) => {
                    return Ok(Judged::Refuse(one_path("region-escape", &path)));
                }
                Err(FacetError::WrongOwner) => {
                    return Ok(Judged::Refuse(one_path("generated-owner-bypass", &path)));
                }
                Err(error) => return Err(facet_error(error)),
            };
            match outcome {
                MergeOutcome::AlreadyTrue => {}
                MergeOutcome::Unchanged { .. } => every_already = false,
                MergeOutcome::Apply { postimage } | MergeOutcome::Merged { postimage } => {
                    every_already = false;
                    current = postimage.image().clone();
                }
                MergeOutcome::Conflict { reason } => {
                    return Ok(Judged::Refuse(one_path(conflict_reason(reason), &path)));
                }
            }
        }
        if current != original {
            changed = true;
            if current.bytes.is_none() {
                files.remove(&path);
            } else {
                files.insert(path, current);
            }
        }
    }
    let kind = if saw_edit && every_already {
        CleanKind::AlreadyTrue
    } else {
        CleanKind::Unchanged
    };
    Ok(Judged::Ready {
        files,
        changed,
        kind,
    })
}

fn group_edits(edits: &[FacetEdit]) -> Vec<(String, Vec<&FacetEdit>)> {
    let mut groups = Vec::<(String, Vec<&FacetEdit>)>::new();
    for edit in edits {
        let path = region_path(&edit.spec).to_owned();
        if let Some((_, grouped)) = groups.iter_mut().find(|(name, _)| name == &path) {
            grouped.push(edit);
        } else {
            groups.push((path, vec![edit]));
        }
    }
    groups
}

fn conflict_reason(reason: ConflictReason) -> &'static str {
    match reason {
        ConflictReason::Overlap | ConflictReason::BothChanged => "overlap",
        ConflictReason::Presence
        | ConflictReason::Mode
        | ConflictReason::NonText
        | ConflictReason::WorkBound => "conflict",
    }
}

fn facet_error(error: FacetError) -> TransactError {
    TransactError::Facet(error.to_string())
}

fn one_path(reason: &'static str, path: &str) -> check::Refusal {
    check::Refusal {
        reason,
        paths: BTreeSet::from([path.to_owned()]),
    }
}

fn outcome_refusal(request: &RepoRequest, refusal: check::Refusal) -> RepoOutcome {
    RepoOutcome::Refused {
        repository: request.repository.clone(),
        reason: refusal.reason,
        paths: refusal.paths,
    }
}

fn refused(
    request: &RepoRequest,
    reason: &'static str,
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> RepoOutcome {
    outcome_refusal(request, one_set(reason, paths))
}

fn one_set(
    reason: &'static str,
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> check::Refusal {
    check::Refusal {
        reason,
        paths: paths
            .into_iter()
            .map(|path| path.as_ref().to_owned())
            .collect(),
    }
}

fn refused_rollback(reason: &'static str, repository: &str) -> Rollback {
    Rollback::Refused {
        reason,
        paths: BTreeSet::from([repository.to_owned()]),
    }
}

#[cfg(test)]
#[path = "transact_fixture.rs"]
pub(crate) mod fixture;
#[cfg(test)]
#[path = "transact_conflict_tests.rs"]
mod transact_conflict_tests;
#[cfg(test)]
#[path = "transact_gate_tests.rs"]
mod transact_gate_tests;
#[cfg(test)]
#[path = "transact_recover_tests.rs"]
mod transact_recover_tests;
#[cfg(test)]
#[path = "transact_tests.rs"]
mod transact_tests;
