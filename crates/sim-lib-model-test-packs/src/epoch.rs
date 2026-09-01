use crate::{ClosureFile, PackError, PackManifest, SourceObject, validate_relative};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, process::Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureEpoch {
    pub id: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl FixtureEpoch {
    /// Reads blobs from the Git object database, never from the mutable checkout.
    pub fn import(
        root: &Path,
        source: &SourceObject,
        closure: &[ClosureFile],
    ) -> Result<Self, PackError> {
        let actual_tree = git_text(root, &["rev-parse", &format!("{}^{{tree}}", source.commit)])?;
        if actual_tree.trim() != source.tree {
            return Err(PackError::ObjectMismatch("tree".into()));
        }
        let mut files = BTreeMap::new();
        for expected in closure {
            validate_relative(&expected.path)?;
            let mode = git_text(root, &["ls-tree", &source.commit, "--", &expected.path])?;
            if mode.starts_with("120000 ") {
                return Err(PackError::Symlink);
            }
            if !mode.starts_with("100") {
                return Err(PackError::ObjectMissing(expected.path.clone()));
            }
            let blob = git_text(
                root,
                &["rev-parse", &format!("{}:{}", source.commit, expected.path)],
            )?;
            if blob.trim() != expected.blob {
                return Err(PackError::ObjectMismatch(expected.path.clone()));
            }
            files.insert(
                expected.path.clone(),
                git_bytes(
                    root,
                    &["show", &format!("{}:{}", source.commit, expected.path)],
                )?,
            );
        }
        let id = epoch_id(source, &files);
        Ok(Self { id, files })
    }

    pub fn verify_regeneration(
        &self,
        root: &Path,
        source: &SourceObject,
        closure: &[ClosureFile],
    ) -> Result<(), PackError> {
        if *self == Self::import(root, source, closure)? {
            Ok(())
        } else {
            Err(PackError::Determinism)
        }
    }
}

fn epoch_id(source: &SourceObject, files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h = Sha256::new();
    for value in [&source.repository, &source.commit, &source.tree] {
        h.update((value.len() as u64).to_be_bytes());
        h.update(value);
    }
    for (path, bytes) in files {
        h.update((path.len() as u64).to_be_bytes());
        h.update(path);
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }
    format!("sha256:{:x}", h.finalize())
}
fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, PackError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|_| PackError::Repository)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(PackError::Repository)
    }
}
fn git_text(root: &Path, args: &[&str]) -> Result<String, PackError> {
    String::from_utf8(git_bytes(root, args)?).map_err(|_| PackError::Repository)
}

pub fn validate_epoch(manifest: &PackManifest, epoch: &FixtureEpoch) -> Result<(), PackError> {
    manifest.validate_shape()?;
    if manifest.bounds.max_tasks
        < manifest
            .selections
            .iter()
            .map(|s| s.task_revisions.len() as u32)
            .max()
            .unwrap_or(0)
    {
        return Err(PackError::Bounds);
    }
    let bytes: u64 = epoch.files.values().map(|v| v.len() as u64).sum();
    if bytes > manifest.bounds.max_input_bytes {
        return Err(PackError::Bounds);
    }
    Ok(())
}
