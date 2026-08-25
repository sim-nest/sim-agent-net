//! Data-only roadmap implementer conduct and strict proposal admission.

use std::collections::BTreeSet;

use crate::{MutationError, PortableImage, SealedMutationPlan, StructuralEdit};

pub const DEFAULT_IMPLEMENTER_TOPOLOGY: &str =
    include_str!("../catalog/roadmap-implementer-v1.simtopo");
pub const IMPLEMENTER_EXTENSION_TARGET: &str = "roadmap/implementer-v1";
pub const IMPLEMENTER_REPLY_SHAPE: &str = "roadmap/ImplementerReply-v1";
pub const IMPLEMENTER_RESULT_SHAPE: &str = "roadmap/SealedMutationPlan-v1";
pub const MAX_MUTATION_FILES: usize = 64;

/// Grounded, bounded material rendered onto the strict BRIDGE face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplementerFace {
    pub grounded_leaf: String,
    pub guide_examples: Vec<(String, String)>,
    pub source_deck: String,
    pub proof_catalog: Vec<String>,
    pub allowed_roots: Vec<String>,
    pub max_files: usize,
    pub max_bytes: usize,
    pub prior_rejection: Option<ProposalRejection>,
}

impl ImplementerFace {
    pub fn render_bridge(&self) -> String {
        fn field(out: &mut String, name: &str, value: &str) {
            out.push_str(&format!(
                "{name}-bytes={}\n{name}=<untrusted-source>{value}</untrusted-source>\n",
                value.len()
            ));
        }
        let mut out = String::from("BRIDGE roadmap/implementer-face-v1\n");
        field(&mut out, "grounded-leaf", &self.grounded_leaf);
        for (label, snippet) in &self.guide_examples {
            field(&mut out, &format!("guide-{label}"), snippet);
        }
        field(&mut out, "source-deck", &self.source_deck);
        field(&mut out, "proof-catalog", &self.proof_catalog.join("\n"));
        field(&mut out, "allowed-roots", &self.allowed_roots.join("\n"));
        out.push_str(&format!(
            "mutation-ceilings=files:{} bytes:{}\n",
            self.max_files, self.max_bytes
        ));
        field(
            &mut out,
            "prior-rejection",
            self.prior_rejection
                .as_ref()
                .map_or("none", ProposalRejection::code),
        );
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalRationale {
    pub text: String,
    pub guide_labels: Vec<String>,
    pub promise_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationProposal {
    pub edits: Vec<StructuralEdit>,
    pub rationale: ProposalRationale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImplementerReply {
    MutationProposal(MutationProposal),
    NeedsRefinement { reason: String },
    Blocked { reason: String },
}

/// Status prose is deliberately separate from the typed reply and is never read by admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplementerPacket {
    pub reply: ImplementerReply,
    pub status_prose: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalRejection {
    EmptyRationale,
    UnknownGuideLabel(String),
    UnknownPromise(String),
    FileCeiling,
    ByteCeiling,
    OutsideAllowedRoots(String),
    ProtectedPath(String),
    GeneratedPath(String),
    BinaryContent(String),
    ExecutableWidening(String),
    Structural(String),
}

impl ProposalRejection {
    pub fn code(&self) -> &str {
        match self {
            Self::EmptyRationale => "empty-rationale",
            Self::UnknownGuideLabel(_) => "unknown-guide-label",
            Self::UnknownPromise(_) => "unknown-promise",
            Self::FileCeiling => "file-ceiling",
            Self::ByteCeiling => "byte-ceiling",
            Self::OutsideAllowedRoots(_) => "outside-allowed-roots",
            Self::ProtectedPath(_) => "protected-path",
            Self::GeneratedPath(_) => "generated-path",
            Self::BinaryContent(_) => "binary-content",
            Self::ExecutableWidening(_) => "executable-widening",
            Self::Structural(_) => "structural",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    Sealed(SealedMutationPlan),
    NeedsRefinement(String),
    Blocked(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalPolicy {
    pub allowed_roots: Vec<String>,
    pub generated_paths: BTreeSet<String>,
    pub guide_labels: BTreeSet<String>,
    pub promise_ids: BTreeSet<String>,
    pub max_files: usize,
    pub max_bytes: usize,
}

/// Deterministically preflights a typed reply. It has no workspace or mutation adapter.
pub fn admit_implementer_reply(
    packet: ImplementerPacket,
    policy: &ProposalPolicy,
) -> Result<Admission, ProposalRejection> {
    let proposal = match packet.reply {
        ImplementerReply::NeedsRefinement { reason } => {
            return Ok(Admission::NeedsRefinement(reason));
        }
        ImplementerReply::Blocked { reason } => return Ok(Admission::Blocked(reason)),
        ImplementerReply::MutationProposal(proposal) => proposal,
    };
    if proposal.rationale.text.trim().is_empty() {
        return Err(ProposalRejection::EmptyRationale);
    }
    for label in &proposal.rationale.guide_labels {
        if !policy.guide_labels.contains(label) {
            return Err(ProposalRejection::UnknownGuideLabel(label.clone()));
        }
    }
    for promise in &proposal.rationale.promise_ids {
        if !policy.promise_ids.contains(promise) {
            return Err(ProposalRejection::UnknownPromise(promise.clone()));
        }
    }
    if proposal.edits.len() > policy.max_files.min(MAX_MUTATION_FILES) {
        return Err(ProposalRejection::FileCeiling);
    }
    for edit in &proposal.edits {
        let allowed = policy.allowed_roots.iter().any(|root| {
            edit.path == *root
                || edit
                    .path
                    .strip_prefix(root)
                    .is_some_and(|p| p.starts_with('/'))
        });
        if !allowed {
            return Err(ProposalRejection::OutsideAllowedRoots(edit.path.clone()));
        }
        if edit.path.ends_with("ROADMAP.md") || edit.path.contains("docs/workbench/") {
            return Err(ProposalRejection::ProtectedPath(edit.path.clone()));
        }
        if policy.generated_paths.contains(&edit.path) {
            return Err(ProposalRejection::GeneratedPath(edit.path.clone()));
        }
        if edit
            .postimage
            .bytes
            .as_ref()
            .is_some_and(|bytes| bytes.contains(&0))
        {
            return Err(ProposalRejection::BinaryContent(edit.path.clone()));
        }
        if matches!((edit.preimage.mode, edit.postimage.mode), (Some(before), Some(after)) if before & 0o111 == 0 && after & 0o111 != 0)
        {
            return Err(ProposalRejection::ExecutableWidening(edit.path.clone()));
        }
    }
    let plan = SealedMutationPlan::seal(proposal.edits)
        .map_err(|error| ProposalRejection::Structural(structural_code(&error).into()))?;
    if plan.total_postimage_bytes > policy.max_bytes {
        return Err(ProposalRejection::ByteCeiling);
    }
    Ok(Admission::Sealed(plan))
}

fn structural_code(error: &MutationError) -> &'static str {
    match error {
        MutationError::InvalidPath => "invalid-path",
        MutationError::DuplicatePath => "duplicate-path",
        MutationError::ParentChildCollision => "parent-child-collision",
        MutationError::InvalidImage => "invalid-image",
        MutationError::InvalidMode => "invalid-mode",
        MutationError::ByteLimit => "byte-limit",
        _ => "invalid-mutation",
    }
}

/// Only predeclared, networkless, read-only specimens can supply observations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationSpecimen {
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub read_roots: Vec<String>,
    pub proof_leaf: Option<String>,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationRequest {
    pub specimen: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub network: bool,
    pub write_mounts: Vec<String>,
    pub max_output_bytes: usize,
}

pub fn admit_observation<'a>(
    request: &ObservationRequest,
    catalog: &'a [ObservationSpecimen],
) -> Result<&'a ObservationSpecimen, &'static str> {
    let specimen = catalog
        .iter()
        .find(|item| item.name == request.specimen)
        .ok_or("unknown-specimen")?;
    if request.argv != specimen.argv {
        return Err("arbitrary-argv");
    }
    if request.cwd != specimen.cwd {
        return Err("undeclared-cwd");
    }
    if request.network {
        return Err("network-forbidden");
    }
    if !request.write_mounts.is_empty() {
        return Err("write-mount-forbidden");
    }
    if request.max_output_bytes == 0 || request.max_output_bytes > specimen.max_output_bytes {
        return Err("output-limit-exceeded");
    }
    if specimen.proof_leaf.is_some() {
        return Err("proof-leaf-is-not-an-implementation-tool");
    }
    Ok(specimen)
}

pub fn text_file(bytes: impl Into<Vec<u8>>, mode: u32) -> PortableImage {
    PortableImage::file(bytes, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ProposalPolicy {
        ProposalPolicy {
            allowed_roots: vec!["crates/widget".into()],
            generated_paths: ["crates/widget/generated.rs".into()].into(),
            guide_labels: ["example-safe".into()].into(),
            promise_ids: ["P1".into()].into(),
            max_files: 4,
            max_bytes: 128,
        }
    }
    fn proposal(path: &str) -> MutationProposal {
        MutationProposal {
            edits: vec![StructuralEdit {
                path: path.into(),
                preimage: text_file("old", 0o644),
                postimage: text_file("new", 0o644),
            }],
            rationale: ProposalRationale {
                text: "Use the grounded contract".into(),
                guide_labels: vec!["example-safe".into()],
                promise_ids: vec!["P1".into()],
            },
        }
    }
    fn packet(proposal: MutationProposal) -> ImplementerPacket {
        ImplementerPacket {
            reply: ImplementerReply::MutationProposal(proposal),
            status_prose: "I already applied this and tests pass".into(),
        }
    }

    #[test]
    fn replay_seals_the_same_plan_and_status_prose_has_no_authority() {
        let first =
            admit_implementer_reply(packet(proposal("crates/widget/src/lib.rs")), &policy())
                .unwrap();
        let second =
            admit_implementer_reply(packet(proposal("crates/widget/src/lib.rs")), &policy())
                .unwrap();
        assert_eq!(first, second);
        assert!(matches!(first, Admission::Sealed(_)));
    }

    #[test]
    fn hostile_mutations_fail_before_a_plan_exists() {
        for (path, code) in [
            ("../escape", "outside-allowed-roots"),
            ("/absolute", "outside-allowed-roots"),
            ("crates/widget/generated.rs", "generated-path"),
            ("crates/widget/docs/workbench/X.md", "protected-path"),
        ] {
            assert_eq!(
                admit_implementer_reply(packet(proposal(path)), &policy())
                    .unwrap_err()
                    .code(),
                code
            );
        }
        let mut duplicate = proposal("crates/widget/src/lib.rs");
        duplicate.edits.push(duplicate.edits[0].clone());
        assert_eq!(
            admit_implementer_reply(packet(duplicate), &policy()).unwrap_err(),
            ProposalRejection::Structural("duplicate-path".into())
        );
        let mut binary = proposal("crates/widget/src/lib.rs");
        binary.edits[0].postimage = text_file([0, 1], 0o644);
        assert_eq!(
            admit_implementer_reply(packet(binary), &policy())
                .unwrap_err()
                .code(),
            "binary-content"
        );
        let mut executable = proposal("crates/widget/src/lib.rs");
        executable.edits[0].postimage.mode = Some(0o755);
        assert_eq!(
            admit_implementer_reply(packet(executable), &policy())
                .unwrap_err()
                .code(),
            "executable-widening"
        );
    }

    #[test]
    fn observation_is_exact_networkless_and_never_a_proof_leaf() {
        let catalog = [ObservationSpecimen {
            name: "inspect-source".into(),
            argv: vec!["inspect".into()],
            cwd: "crates/widget".into(),
            read_roots: vec!["src".into()],
            proof_leaf: None,
            max_output_bytes: 4096,
        }];
        let mut request = ObservationRequest {
            specimen: "inspect-source".into(),
            argv: vec!["inspect".into()],
            cwd: "crates/widget".into(),
            network: false,
            write_mounts: vec![],
            max_output_bytes: 4096,
        };
        assert!(admit_observation(&request, &catalog).is_ok());
        request.network = true;
        assert_eq!(
            admit_observation(&request, &catalog),
            Err("network-forbidden")
        );
    }

    #[test]
    fn conduct_has_every_bounded_node_and_no_effect_node() {
        for node in [
            "receive", "inspect", "propose", "validate", "review", "revise", "finish",
        ] {
            assert!(DEFAULT_IMPLEMENTER_TOPOLOGY.contains(&format!("node {node} ")));
        }
        assert!(DEFAULT_IMPLEMENTER_TOPOLOGY.contains("max-visits="));
        assert!(!DEFAULT_IMPLEMENTER_TOPOLOGY.contains("verb=effect"));
        assert!(!DEFAULT_IMPLEMENTER_TOPOLOGY.contains("verb=tool"));
    }
}
// conformance: exact data-only roadmap implementer conduct.
