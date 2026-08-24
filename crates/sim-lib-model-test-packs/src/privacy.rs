use crate::PackManifest;
use sim_study_core::{EvidenceClass, FieldClass};
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackPrivacy {
    Public,
    PrivateLocal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicPackRecord {
    pub id: String,
    pub revision: String,
    pub evidence_class: EvidenceClass,
    pub source_ids: Vec<String>,
    pub grader_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPrivatePack {
    pub manifest: PackManifest,
    pub private_bytes: Vec<u8>,
}

pub trait PrivatePackLoader {
    /// Implemented by the control plane. Public crates never discover private paths.
    fn load_private(&self, manifest_id: &str) -> Result<LoadedPrivatePack, PackError>;
}

impl PackManifest {
    pub fn export_public(&self) -> Result<PublicPackRecord, PackError> {
        if self.privacy != PackPrivacy::Public
            || self.evidence_class == EvidenceClass::PrivateLocal
            || !self.hidden_grader_ids.is_empty()
        {
            return Err(PackError::PrivateExport);
        }
        if Self::field_classes()
            .iter()
            .any(|(_, class)| *class == FieldClass::SecretForbidden)
        {
            return Err(PackError::SecretForbidden);
        }
        Ok(PublicPackRecord {
            id: self.id.clone(),
            revision: self.revision.clone(),
            evidence_class: self.evidence_class,
            source_ids: self
                .sources
                .iter()
                .map(|s| format!("{}:{}:{}", s.repository, s.commit, s.tree))
                .collect(),
            grader_ids: self.grader_ids.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    Schema,
    Id,
    Source,
    License,
    Lockfile,
    Bounds,
    UnsafePath,
    Symlink,
    ObjectMissing(String),
    ObjectMismatch(String),
    Duplicate(&'static str),
    Missing(&'static str),
    Selection,
    PrivateExport,
    SecretForbidden,
    Determinism,
    TestFailure(String),
    Repository,
}
impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model-test pack rejected: {self:?}")
    }
}
impl Error for PackError {}
