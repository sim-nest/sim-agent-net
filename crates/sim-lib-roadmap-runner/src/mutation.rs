use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{ResumeDecision, classify_plan};

pub const DEFAULT_MAX_MUTATION_BYTES: usize = 32 * 1024 * 1024;

/// Complete portable file state. `None` is absence; bytes are deliberately not UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortableImage {
    pub bytes: Option<Vec<u8>>,
    pub mode: Option<u32>,
}

impl PortableImage {
    pub fn absent() -> Self {
        Self {
            bytes: None,
            mode: None,
        }
    }
    pub fn file(bytes: impl Into<Vec<u8>>, mode: u32) -> Self {
        Self {
            bytes: Some(bytes.into()),
            mode: Some(mode),
        }
    }
    fn validate(&self) -> Result<(), MutationError> {
        if self.bytes.is_some() != self.mode.is_some() {
            return Err(MutationError::InvalidImage);
        }
        if self.mode.is_some_and(|m| m & !0o777 != 0) {
            return Err(MutationError::InvalidMode);
        }
        Ok(())
    }
}

/// Input edits may arrive unordered, but must already contain exact full images.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralEdit {
    pub path: String,
    pub preimage: PortableImage,
    pub postimage: PortableImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedEntry {
    pub path: String,
    pub preimage: PortableImage,
    pub postimage: PortableImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedMutationPlan {
    pub id: [u8; 32],
    pub entries: Vec<SealedEntry>,
    pub total_postimage_bytes: usize,
}

impl SealedMutationPlan {
    pub fn seal(edits: Vec<StructuralEdit>) -> Result<Self, MutationError> {
        Self::from_images(
            edits
                .into_iter()
                .map(|e| (e.path, e.preimage, e.postimage))
                .collect(),
        )
    }

    pub(crate) fn from_images(
        mut images: Vec<(String, PortableImage, PortableImage)>,
    ) -> Result<Self, MutationError> {
        images.sort_by(|a, b| a.0.cmp(&b.0));
        if images.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(MutationError::DuplicatePath);
        }
        let paths: BTreeSet<_> = images.iter().map(|e| e.0.as_str()).collect();
        for (path, pre, post) in &images {
            validate_relative_path(path)?;
            pre.validate()?;
            post.validate()?;
            let mut parent = Path::new(path).parent();
            while let Some(p) = parent {
                if !p.as_os_str().is_empty() && paths.contains(p.to_str().unwrap_or_default()) {
                    return Err(MutationError::ParentChildCollision);
                }
                parent = p.parent();
            }
        }
        let total = images.iter().try_fold(0usize, |n, e| {
            n.checked_add(e.2.bytes.as_ref().map_or(0, Vec::len))
                .ok_or(MutationError::ByteLimit)
        })?;
        if total > DEFAULT_MAX_MUTATION_BYTES {
            return Err(MutationError::ByteLimit);
        }
        let entries: Vec<_> = images
            .into_iter()
            .map(|(path, preimage, postimage)| SealedEntry {
                path,
                preimage,
                postimage,
            })
            .collect();
        let id = plan_digest(&entries);
        Ok(Self {
            id,
            entries,
            total_postimage_bytes: total,
        })
    }
}

fn validate_relative_path(path: &str) -> Result<(), MutationError> {
    let p = Path::new(path);
    if path.is_empty() || path.len() > 4096 || p.is_absolute() || path.contains('\\') {
        return Err(MutationError::InvalidPath);
    }
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(MutationError::InvalidPath);
    }
    Ok(())
}

fn plan_digest(entries: &[SealedEntry]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"sim-roadmap-mutation-v1\0");
    for e in entries {
        digest_field(&mut h, e.path.as_bytes());
        digest_image(&mut h, &e.preimage);
        digest_image(&mut h, &e.postimage);
    }
    h.finalize().into()
}

pub(crate) fn encode_plan(plan: &SealedMutationPlan) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"sim-roadmap-mutation-plan-v1\0");
    out.extend_from_slice(&plan.id);
    out.extend_from_slice(&(plan.entries.len() as u32).to_be_bytes());
    for entry in &plan.entries {
        encode_bytes(&mut out, entry.path.as_bytes());
        encode_portable_image(&mut out, &entry.preimage);
        encode_portable_image(&mut out, &entry.postimage);
    }
    out
}

fn encode_portable_image(out: &mut Vec<u8>, image: &PortableImage) {
    match (&image.bytes, image.mode) {
        (Some(bytes), Some(mode)) => {
            out.push(1);
            out.extend_from_slice(&mode.to_be_bytes());
            encode_bytes(out, bytes);
        }
        _ => out.push(0),
    }
}

fn encode_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

pub(crate) fn mutation_id_text(id: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in id {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
fn digest_field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_be_bytes());
    h.update(bytes);
}
fn digest_image(h: &mut Sha256, image: &PortableImage) {
    match (&image.bytes, image.mode) {
        (Some(bytes), Some(mode)) => {
            h.update([1]);
            h.update(mode.to_be_bytes());
            digest_field(h, bytes);
        }
        _ => h.update([0]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationFence {
    Prepared,
    Applying(usize),
    Verifying,
    Committed,
    Ambiguous,
}

pub trait MutationJournal {
    fn put_plan(&mut self, plan: &SealedMutationPlan) -> Result<(), MutationError>;
    fn append_fence(
        &mut self,
        plan_id: [u8; 32],
        fence: MutationFence,
    ) -> Result<(), MutationError>;
}

pub trait MutationWorkspace {
    fn observe(&self, path: &str) -> Result<PortableImage, MutationError>;
    fn scratch_available(&self) -> Result<Option<u64>, MutationError> {
        Ok(None)
    }
    fn apply(
        &mut self,
        entry: &SealedEntry,
        fail: &mut dyn FnMut(Failpoint) -> Result<(), MutationError>,
    ) -> Result<(), MutationError>;
    fn durability(&self) -> Durability {
        Durability::Replace
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    Replace,
    FileAndDirectorySync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Failpoint {
    BeforeObjectPut,
    AfterObjectPut,
    BeforeJournalAppend,
    AfterJournalAppend,
    BeforeTempWrite,
    AfterTempWrite,
    BeforeFlush,
    AfterFlush,
    BeforeReplace,
    AfterReplace,
    BeforeDirectorySync,
    AfterDirectorySync,
    BeforePostimageObservation,
    AfterPostimageObservation,
}

pub struct MutationEngine<W, J> {
    pub workspace: W,
    pub journal: J,
}

impl<W: MutationWorkspace, J: MutationJournal> MutationEngine<W, J> {
    pub fn execute(&mut self, plan: &SealedMutationPlan) -> Result<MutationReceipt, MutationError> {
        self.execute_with(plan, |_| Ok(()))
    }
    pub fn execute_with(
        &mut self,
        plan: &SealedMutationPlan,
        mut fail: impl FnMut(Failpoint) -> Result<(), MutationError>,
    ) -> Result<MutationReceipt, MutationError> {
        let observed = self.preflight(plan)?;
        match classify_plan(plan, &observed) {
            ResumeDecision::Ambiguous { foreign_paths } => {
                return Err(MutationError::Ambiguous { foreign_paths });
            }
            ResumeDecision::Committed => {
                return Ok(MutationReceipt {
                    plan_id: plan.id,
                    durability: self.workspace.durability(),
                });
            }
            ResumeDecision::Apply { .. } => {}
        }
        fail(Failpoint::BeforeObjectPut)?;
        self.journal.put_plan(plan)?;
        fail(Failpoint::AfterObjectPut)?;
        self.fence(plan.id, MutationFence::Prepared, &mut fail)?;
        self.resume_apply(plan, &mut fail)
    }

    pub fn resume(&mut self, plan: &SealedMutationPlan) -> Result<MutationReceipt, MutationError> {
        self.resume_apply(plan, &mut |_| Ok(()))
    }

    fn resume_apply(
        &mut self,
        plan: &SealedMutationPlan,
        fail: &mut dyn FnMut(Failpoint) -> Result<(), MutationError>,
    ) -> Result<MutationReceipt, MutationError> {
        let observed = self.observe_all(plan)?;
        let paths = match classify_plan(plan, &observed) {
            ResumeDecision::Committed => Vec::new(),
            ResumeDecision::Apply { paths } => paths,
            ResumeDecision::Ambiguous { foreign_paths } => {
                self.fence(plan.id, MutationFence::Ambiguous, fail)?;
                return Err(MutationError::Ambiguous { foreign_paths });
            }
        };
        for (index, entry) in plan.entries.iter().enumerate() {
            if !paths.contains(&entry.path) {
                continue;
            }
            self.fence(plan.id, MutationFence::Applying(index), fail)?;
            self.workspace.apply(entry, fail)?;
            fail(Failpoint::BeforePostimageObservation)?;
            let actual = self.workspace.observe(&entry.path)?;
            fail(Failpoint::AfterPostimageObservation)?;
            if actual != entry.postimage {
                return Err(MutationError::PostimageMismatch(entry.path.clone()));
            }
        }
        self.fence(plan.id, MutationFence::Verifying, fail)?;
        match classify_plan(plan, &self.observe_all(plan)?) {
            ResumeDecision::Committed => {
                self.fence(plan.id, MutationFence::Committed, fail)?;
                Ok(MutationReceipt {
                    plan_id: plan.id,
                    durability: self.workspace.durability(),
                })
            }
            ResumeDecision::Ambiguous { foreign_paths } => {
                Err(MutationError::Ambiguous { foreign_paths })
            }
            ResumeDecision::Apply { .. } => Err(MutationError::Incomplete),
        }
    }

    fn fence(
        &mut self,
        id: [u8; 32],
        fence: MutationFence,
        fail: &mut dyn FnMut(Failpoint) -> Result<(), MutationError>,
    ) -> Result<(), MutationError> {
        fail(Failpoint::BeforeJournalAppend)?;
        self.journal.append_fence(id, fence)?;
        fail(Failpoint::AfterJournalAppend)
    }

    fn preflight(
        &self,
        plan: &SealedMutationPlan,
    ) -> Result<BTreeMap<String, PortableImage>, MutationError> {
        let observed = self.observe_all(plan)?;
        if let ResumeDecision::Ambiguous { foreign_paths } = classify_plan(plan, &observed) {
            return Err(MutationError::PreimageMismatch(foreign_paths));
        }
        if self
            .workspace
            .scratch_available()?
            .is_some_and(|n| n < plan.total_postimage_bytes as u64)
        {
            return Err(MutationError::ScratchCapacity);
        }
        Ok(observed)
    }
    pub fn observe_all(
        &self,
        plan: &SealedMutationPlan,
    ) -> Result<BTreeMap<String, PortableImage>, MutationError> {
        plan.entries
            .iter()
            .map(|e| Ok((e.path.clone(), self.workspace.observe(&e.path)?)))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationReceipt {
    pub plan_id: [u8; 32],
    pub durability: Durability,
}

#[derive(Debug, Error)]
pub enum MutationError {
    #[error("invalid repository-relative path")]
    InvalidPath,
    #[error("duplicate mutation path")]
    DuplicatePath,
    #[error("mutation contains a parent/child path collision")]
    ParentChildCollision,
    #[error("file image existence, bytes, and mode disagree")]
    InvalidImage,
    #[error("file mode is not portable")]
    InvalidMode,
    #[error("mutation byte limit exceeded")]
    ByteLimit,
    #[error("insufficient scratch capacity")]
    ScratchCapacity,
    #[error("preimage mismatch: {0:?}")]
    PreimageMismatch(Vec<String>),
    #[error("foreign state makes mutation ambiguous: {foreign_paths:?}")]
    Ambiguous { foreign_paths: Vec<String> },
    #[error("postimage was not observed for {0}")]
    PostimageMismatch(String),
    #[error("mutation remains incomplete")]
    Incomplete,
    #[error("unsupported file kind or symlink at {0}")]
    UnsupportedFileKind(String),
    #[error("mutation I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("injected failure at {0:?}")]
    Injected(Failpoint),
    #[error("journal failure: {0}")]
    Journal(String),
}

include!("mutation/fs_workspace.rs");
// conformance: recoverable exact multi-file mutation.
