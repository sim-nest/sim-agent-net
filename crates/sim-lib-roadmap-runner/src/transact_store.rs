// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! In-memory artifact tree and branch pointer.
//!
//! The store is a model provider. It does not open a checkout, follow a
//! symlink, or replace a file on disk. Commit bytes stay under their id when
//! the branch pointer moves.

use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::PortableImage;
use sim_kernel::{ContentId, Datum, Symbol};

use super::TransactError;

/// Branch name and commit id the model currently names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelTip {
    /// Branch the pointer sits on.
    pub branch: String,
    /// Content id of the commit that pointer names.
    pub head: String,
}

/// Refusal from the model branch pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFault {
    /// The named branch or head is not the pointer.
    WrongHead,
    /// The model work tree has uncommitted paths.
    Dirt,
    /// The commit id could not be formed.
    Identity(String),
}

/// Reads committed model bytes. This is not a filesystem.
pub trait ArtifactModel {
    /// Reports whether that repository still has the revision id.
    fn revision_exists(&self, repository: &str, revision: &str) -> bool;
    /// Returns the committed image, or an absent image when the path is missing.
    fn image_at(&self, repository: &str, revision: &str, path: &str) -> PortableImage;
    /// Returns the full committed tree for one revision.
    fn tree(&self, repository: &str, revision: &str) -> Option<BTreeMap<String, PortableImage>>;
}

/// Moves a model branch pointer. Facet comparison stays with the artifact law.
pub trait GitModel {
    /// Returns the branch pointer, when the repository exists.
    fn tip(&self, repository: &str) -> Option<ModelTip>;
    /// Returns uncommitted paths. An unknown repository is empty.
    fn dirt(&self, repository: &str) -> BTreeSet<String>;
    /// Records one new commit and moves the pointer when `expected` still matches.
    fn commit(
        &mut self,
        repository: &str,
        expected: &ModelTip,
        files: &BTreeMap<String, PortableImage>,
    ) -> Result<ModelTip, GitFault>;
    /// Points the branch at an existing commit. The commit bytes stay in place.
    fn rollback(
        &mut self,
        repository: &str,
        expected: &ModelTip,
        target: &str,
    ) -> Result<ModelTip, GitFault>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Repository {
    branch: String,
    head: String,
    commits: BTreeMap<String, BTreeMap<String, PortableImage>>,
    dirt: BTreeSet<String>,
}

/// One model provider holding independent repository trees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelStore {
    repos: BTreeMap<String, Repository>,
}

impl ModelStore {
    /// Empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a repository whose first commit is the supplied tree.
    pub fn insert_repository(
        &mut self,
        name: impl Into<String>,
        branch: impl Into<String>,
        files: BTreeMap<String, PortableImage>,
    ) -> Result<ModelTip, TransactError> {
        let name = name.into();
        if self.repos.contains_key(&name) {
            return Err(TransactError::Identity(format!("repository {name} exists")));
        }
        let head = commit_identity(None, &files)?;
        let branch = branch.into();
        let tip = ModelTip {
            branch: branch.clone(),
            head: head.clone(),
        };
        self.repos.insert(
            name,
            Repository {
                branch,
                head,
                commits: BTreeMap::from([(tip.head.clone(), files)]),
                dirt: BTreeSet::new(),
            },
        );
        Ok(tip)
    }

    /// Records an uncommitted path. The committed tree stays as it is.
    pub fn mark_dirt(&mut self, repository: &str, path: &str) -> bool {
        let Some(repo) = self.repos.get_mut(repository) else {
            return false;
        };
        repo.dirt.insert(path.to_owned());
        true
    }
}

impl ArtifactModel for ModelStore {
    fn revision_exists(&self, repository: &str, revision: &str) -> bool {
        self.repos
            .get(repository)
            .is_some_and(|repo| repo.commits.contains_key(revision))
    }

    fn image_at(&self, repository: &str, revision: &str, path: &str) -> PortableImage {
        self.repos
            .get(repository)
            .and_then(|repo| repo.commits.get(revision))
            .and_then(|files| files.get(path).cloned())
            .unwrap_or_else(PortableImage::absent)
    }

    fn tree(&self, repository: &str, revision: &str) -> Option<BTreeMap<String, PortableImage>> {
        self.repos
            .get(repository)
            .and_then(|repo| repo.commits.get(revision).cloned())
    }
}

impl GitModel for ModelStore {
    fn tip(&self, repository: &str) -> Option<ModelTip> {
        self.repos.get(repository).map(|repo| ModelTip {
            branch: repo.branch.clone(),
            head: repo.head.clone(),
        })
    }

    fn dirt(&self, repository: &str) -> BTreeSet<String> {
        self.repos
            .get(repository)
            .map(|repo| repo.dirt.clone())
            .unwrap_or_default()
    }

    fn commit(
        &mut self,
        repository: &str,
        expected: &ModelTip,
        files: &BTreeMap<String, PortableImage>,
    ) -> Result<ModelTip, GitFault> {
        let (branch, head) = {
            let Some(repo) = self.repos.get(repository) else {
                return Err(GitFault::WrongHead);
            };
            if repo.branch != expected.branch || repo.head != expected.head {
                return Err(GitFault::WrongHead);
            }
            if !repo.dirt.is_empty() {
                return Err(GitFault::Dirt);
            }
            (repo.branch.clone(), repo.head.clone())
        };
        let id = commit_identity(Some(&head), files).map_err(|error| match error {
            TransactError::Identity(text) | TransactError::Facet(text) => GitFault::Identity(text),
        })?;
        let Some(repo) = self.repos.get_mut(repository) else {
            return Err(GitFault::WrongHead);
        };
        repo.commits.insert(id.clone(), files.clone());
        repo.head = id.clone();
        Ok(ModelTip { branch, head: id })
    }

    fn rollback(
        &mut self,
        repository: &str,
        expected: &ModelTip,
        target: &str,
    ) -> Result<ModelTip, GitFault> {
        let Some(repo) = self.repos.get_mut(repository) else {
            return Err(GitFault::WrongHead);
        };
        if repo.branch != expected.branch || repo.head != expected.head {
            return Err(GitFault::WrongHead);
        }
        if !repo.dirt.is_empty() {
            return Err(GitFault::Dirt);
        }
        if !repo.commits.contains_key(target) {
            return Err(GitFault::WrongHead);
        }
        repo.head = target.to_owned();
        Ok(ModelTip {
            branch: repo.branch.clone(),
            head: repo.head.clone(),
        })
    }
}

fn commit_identity(
    parent: Option<&str>,
    files: &BTreeMap<String, PortableImage>,
) -> Result<String, TransactError> {
    let entries = files
        .iter()
        .map(|(path, image)| Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "ModelFileV1"),
            fields: vec![
                (Symbol::new("path"), Datum::String(path.clone())),
                (
                    Symbol::new("mode"),
                    Datum::String(image.mode.unwrap_or(0).to_string()),
                ),
                (
                    Symbol::new("bytes"),
                    match &image.bytes {
                        Some(bytes) => Datum::Bytes(bytes.clone()),
                        None => Datum::Nil,
                    },
                ),
            ],
        })
        .collect();
    let parent = match parent {
        Some(parent) => Datum::String(parent.to_owned()),
        None => Datum::Nil,
    };
    let id = Datum::Node {
        tag: Symbol::qualified("roadmap-runner", "ModelCommitV1"),
        fields: vec![
            (Symbol::new("parent"), parent),
            (Symbol::new("files"), Datum::List(entries)),
        ],
    }
    .content_id()
    .map_err(|error| TransactError::Identity(error.to_string()))?;
    Ok(content_id_text(&id))
}

fn content_id_text(id: &ContentId) -> String {
    let bytes = id
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}:{bytes}", id.algorithm)
}
