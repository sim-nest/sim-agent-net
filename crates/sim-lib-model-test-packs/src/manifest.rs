use crate::{PackError, PackPrivacy, SelectionRevision};
use sim_study_core::{EvidenceClass, FieldClass};
use std::collections::BTreeSet;

pub const PACK_SCHEMA: &str = "sim.model-test-pack/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceObject {
    pub repository: String,
    pub commit: String,
    pub tree: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureFile {
    pub path: String,
    pub blob: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseGrant {
    pub expression: String,
    pub notice_file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkBounds {
    pub max_tasks: u32,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_tool_calls: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub schema: String,
    pub id: String,
    pub revision: String,
    pub families: Vec<String>,
    pub facets: Vec<String>,
    pub evidence_class: EvidenceClass,
    pub privacy: PackPrivacy,
    pub sources: Vec<SourceObject>,
    pub licenses: Vec<LicenseGrant>,
    pub toolchain: String,
    pub lockfile: ClosureFile,
    pub closure: Vec<ClosureFile>,
    pub public_tests: Vec<String>,
    pub grader_ids: Vec<String>,
    pub hidden_grader_ids: Vec<String>,
    pub bounds: WorkBounds,
    pub selections: Vec<SelectionRevision>,
}

impl PackManifest {
    pub fn field_classes() -> &'static [(&'static str, FieldClass)] {
        &[
            ("schema", FieldClass::Public),
            ("id", FieldClass::Public),
            ("revision", FieldClass::DigestOnly),
            ("families", FieldClass::Public),
            ("facets", FieldClass::Public),
            ("evidence_class", FieldClass::Public),
            ("privacy", FieldClass::Public),
            ("sources", FieldClass::DigestOnly),
            ("licenses", FieldClass::Public),
            ("toolchain", FieldClass::Public),
            ("lockfile", FieldClass::DigestOnly),
            ("closure", FieldClass::DigestOnly),
            ("public_tests", FieldClass::Public),
            ("grader_ids", FieldClass::DigestOnly),
            ("hidden_grader_ids", FieldClass::PrivateLocal),
            ("bounds", FieldClass::Public),
            ("selections", FieldClass::Public),
        ]
    }

    pub fn validate_shape(&self) -> Result<(), PackError> {
        if self.schema != PACK_SCHEMA {
            return Err(PackError::Schema);
        }
        bounded_id(&self.id)?;
        bounded_id(&self.revision)?;
        unique_nonempty(&self.families, "families")?;
        unique_nonempty(&self.facets, "facets")?;
        unique_nonempty(&self.grader_ids, "grader ids")?;
        if self.sources.is_empty() || self.licenses.is_empty() || self.closure.is_empty() {
            return Err(PackError::Missing("sources, licenses, or closure"));
        }
        if self.bounds.max_tasks == 0
            || self.bounds.max_input_bytes == 0
            || self.bounds.max_output_bytes == 0
            || self.bounds.max_tool_calls == 0
        {
            return Err(PackError::Bounds);
        }
        validate_relative(&self.lockfile.path)?;
        let mut paths = BTreeSet::new();
        for file in &self.closure {
            validate_relative(&file.path)?;
            if !paths.insert(&file.path) {
                return Err(PackError::Duplicate("closure path"));
            }
            if file.blob.is_empty() {
                return Err(PackError::Missing("blob id"));
            }
        }
        if !paths.contains(&self.lockfile.path) || self.lockfile.blob.is_empty() {
            return Err(PackError::Lockfile);
        }
        for source in &self.sources {
            if source.repository.contains('@') || source.commit.len() < 7 || source.tree.is_empty()
            {
                return Err(PackError::Source);
            }
        }
        for license in &self.licenses {
            if license.expression.trim().is_empty() {
                return Err(PackError::License);
            }
            if let Some(path) = &license.notice_file {
                validate_relative(path)?;
            }
        }
        let mut names = BTreeSet::new();
        for selection in &self.selections {
            selection.validate()?;
            if !names.insert(&selection.name) {
                return Err(PackError::Duplicate("selection"));
            }
        }
        Ok(())
    }
}

fn bounded_id(value: &str) -> Result<(), PackError> {
    if value.is_empty() || value.len() > 256 || value.trim() != value {
        Err(PackError::Id)
    } else {
        Ok(())
    }
}
fn unique_nonempty(values: &[String], name: &'static str) -> Result<(), PackError> {
    if values.is_empty() || values.iter().any(String::is_empty) {
        return Err(PackError::Missing(name));
    }
    let unique: BTreeSet<_> = values.iter().collect();
    if unique.len() != values.len() {
        Err(PackError::Duplicate(name))
    } else {
        Ok(())
    }
}
pub(crate) fn validate_relative(path: &str) -> Result<(), PackError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        Err(PackError::UnsafePath)
    } else {
        Ok(())
    }
}
