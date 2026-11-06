// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Landing answers over a model store.
//!
//! Plan observes. Apply commits through the shared facet transaction. A cached
//! answer is current only while the branch still names the head it recorded.

use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::PortableImage;

use crate::transact::{
    ArtifactModel, CleanKind, FacetPreview, GitModel, ModelBatch, ModelTip, RepoOutcome,
    RepoRequest, Rollback, TransactError, preview_repository, rollback_model, transact_facets,
};

/// Remembered landing answer for one repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandCache {
    answers: BTreeMap<String, CachedAnswer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CachedAnswer {
    head: String,
    text: String,
}

impl LandCache {
    /// An empty cache. It holds no answer until a landing call records one.
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&mut self, repository: &str, head: &str, text: impl Into<String>) {
        self.answers.insert(
            repository.to_owned(),
            CachedAnswer {
                head: head.to_owned(),
                text: text.into(),
            },
        );
    }
}

/// Observation of one repository before any commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LandPlan {
    /// Apply would record one commit on this tip.
    WouldCommit {
        /// Repository name.
        repository: String,
        /// Pointer before the commit.
        tip: ModelTip,
        /// Tree the commit would store.
        files: BTreeMap<String, PortableImage>,
    },
    /// Apply would leave the pointer where it is.
    Clean {
        /// Repository name.
        repository: String,
        /// Pointer that stays.
        tip: ModelTip,
        /// Why no commit is needed.
        kind: CleanKind,
    },
    /// Apply would write nothing.
    Refused {
        /// Repository name.
        repository: String,
        /// Stable reason.
        reason: &'static str,
        /// Paths or the repository named by the refusal.
        paths: BTreeSet<String>,
    },
}

/// Branch pointer and uncommitted paths, or a missing repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LandStatus {
    /// The model holds this repository.
    Present {
        /// Repository name.
        repository: String,
        /// Current pointer.
        tip: ModelTip,
        /// Uncommitted paths.
        dirty: BTreeSet<String>,
    },
    /// The model has no such repository.
    Missing {
        /// Repository name.
        repository: String,
    },
}

/// A cached explanation, a freshness gap, or no recorded answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LandWhy {
    /// The recorded answer still names the current head.
    Current {
        /// Repository name.
        repository: String,
        /// Head the answer was recorded against.
        head: String,
        /// Recorded explanation.
        text: String,
    },
    /// The pointer moved after the answer was recorded.
    Stale {
        /// Repository name.
        repository: String,
        /// Head the answer names.
        cached_head: String,
        /// Head the pointer names now.
        current_head: String,
    },
    /// No landing call has recorded an answer.
    Unknown {
        /// Repository name.
        repository: String,
    },
}

/// Observes one repository and records that answer at the current head.
pub fn land_plan<S>(
    store: &S,
    cache: &mut LandCache,
    request: &RepoRequest,
) -> Result<LandPlan, TransactError>
where
    S: ArtifactModel + GitModel,
{
    let plan = match preview_repository(store, request)? {
        FacetPreview::WouldCommit {
            repository,
            tip,
            files,
        } => {
            cache.record(&repository, &tip.head, "would-commit");
            LandPlan::WouldCommit {
                repository,
                tip,
                files,
            }
        }
        FacetPreview::Clean {
            repository,
            tip,
            kind,
        } => {
            cache.record(&repository, &tip.head, clean_text(kind));
            LandPlan::Clean {
                repository,
                tip,
                kind,
            }
        }
        FacetPreview::Refused {
            repository,
            reason,
            paths,
        } => {
            if let Some(tip) = store.tip(&repository) {
                cache.record(&repository, &tip.head, format!("refused:{reason}"));
            }
            LandPlan::Refused {
                repository,
                reason,
                paths,
            }
        }
    };
    Ok(plan)
}

/// Commits each repository through the facet transaction and records the result.
pub fn land_apply<S>(
    store: &mut S,
    cache: &mut LandCache,
    requests: &[RepoRequest],
) -> Result<ModelBatch, TransactError>
where
    S: ArtifactModel + GitModel,
{
    let batch = transact_facets(store, requests)?;
    for outcome in batch.outcomes() {
        note_outcome(cache, store, outcome);
    }
    Ok(batch)
}

/// Reads the branch pointer. It does not compare facets.
pub fn land_status<S>(store: &S, repository: &str) -> LandStatus
where
    S: GitModel,
{
    match store.tip(repository) {
        Some(tip) => LandStatus::Present {
            repository: repository.to_owned(),
            dirty: store.dirt(repository),
            tip,
        },
        None => LandStatus::Missing {
            repository: repository.to_owned(),
        },
    }
}

/// Returns the recorded answer when its head is still current.
///
/// A moved head is a freshness gap. This call does not recompute the answer.
pub fn land_why<S>(store: &S, cache: &LandCache, repository: &str) -> LandWhy
where
    S: GitModel,
{
    let Some(answer) = cache.answers.get(repository) else {
        return LandWhy::Unknown {
            repository: repository.to_owned(),
        };
    };
    match store.tip(repository) {
        Some(tip) if tip.head == answer.head => LandWhy::Current {
            repository: repository.to_owned(),
            head: answer.head.clone(),
            text: answer.text.clone(),
        },
        Some(tip) => LandWhy::Stale {
            repository: repository.to_owned(),
            cached_head: answer.head.clone(),
            current_head: tip.head,
        },
        None => LandWhy::Stale {
            repository: repository.to_owned(),
            cached_head: answer.head.clone(),
            current_head: String::new(),
        },
    }
}

/// Moves the branch pointer and records that answer at the restored head.
pub fn land_recover<S>(
    store: &mut S,
    cache: &mut LandCache,
    repository: &str,
    expected: &ModelTip,
    target: &str,
) -> Result<Rollback, TransactError>
where
    S: GitModel,
{
    let rollback = rollback_model(store, repository, expected, target)?;
    if let Rollback::Restored { tip } = &rollback {
        cache.record(repository, &tip.head, "restored");
    }
    Ok(rollback)
}

fn clean_text(kind: CleanKind) -> &'static str {
    match kind {
        CleanKind::AlreadyTrue => "already-true",
        CleanKind::Unchanged => "unchanged",
    }
}

fn note_outcome<S>(cache: &mut LandCache, store: &S, outcome: &RepoOutcome)
where
    S: GitModel,
{
    let repository = match outcome {
        RepoOutcome::Applied { repository, .. }
        | RepoOutcome::Clean { repository, .. }
        | RepoOutcome::Refused { repository, .. } => repository.as_str(),
    };
    let Some(tip) = store.tip(repository) else {
        return;
    };
    let text = match outcome {
        RepoOutcome::Applied { .. } => "applied".to_owned(),
        RepoOutcome::Clean { kind, .. } => clean_text(*kind).to_owned(),
        RepoOutcome::Refused { reason, .. } => format!("refused:{reason}"),
    };
    cache.record(repository, &tip.head, text);
}

#[cfg(test)]
#[path = "land_tests.rs"]
mod land_tests;
