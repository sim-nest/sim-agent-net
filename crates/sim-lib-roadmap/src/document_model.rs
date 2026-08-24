use std::collections::BTreeMap;

/// Exact byte range in an imported legacy document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRange {
    pub document: String,
    pub start: usize,
    pub end: usize,
}

/// Runtime state observed in v3 text. It is deliberately not part of a native
/// roadmap's semantic identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyObservation {
    pub state: Option<String>,
    pub next: Option<String>,
    pub phase_status: BTreeMap<String, String>,
    pub checked_tasks: BTreeMap<String, Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub relation: OriginRelation,
    pub path: String,
    pub fragment: Option<String>,
    pub content_id: String,
    pub span: SourceRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginRelation {
    Source,
    Review,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseEdge {
    pub target: String,
    pub kind: EdgeKind,
    pub span: SourceRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    Requires,
    After,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub text: String,
    pub span: SourceRange,
}

/// Legacy prose is guidance with an explicit lack of grounding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideEntry {
    pub text: String,
    pub language: Option<String>,
    pub span: SourceRange,
    pub grounded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePhase {
    pub id: String,
    pub title: String,
    pub span: SourceRange,
    pub dependencies: Vec<PhaseEdge>,
    pub checkpoints: Vec<Checkpoint>,
    pub origins: Vec<Origin>,
    pub guides: Vec<GuideEntry>,
    pub children: Vec<NativePhase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRoadmap {
    pub title: String,
    pub root: NativePhase,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedRoadmap {
    pub roadmap: NativeRoadmap,
    pub observation: LegacyObservation,
}

impl NativeRoadmap {
    /// Stable semantic identity excludes every imported runtime observation.
    pub fn semantic_id(&self) -> String {
        let text = crate::render_native(self);
        format!("roadmap:{:016x}", stable_hash(text.as_bytes()))
    }
}

pub(crate) fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
