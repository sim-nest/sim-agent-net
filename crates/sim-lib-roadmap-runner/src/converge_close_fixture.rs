// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sim_kernel::{ContentId, Symbol};
use sim_lib_exec::SandboxLimits;
use sim_lib_journal::{JournalHead, MemoryBackend};

use super::fixture::{claim, complete};
use super::{AttachStep, Proposal, ProposalBody, Revision, Step, open};
use crate::{
    AttachError, CommandProof, ExecutionJournal, ExecutionPins, ExecutionRecord, Fence, Limits,
    ProofCatalog, ProofLeaf, ProofOutputId, StructuredExpectation,
};

pub(crate) fn open_journal() -> (ExecutionJournal<MemoryBackend>, JournalHead) {
    let journal = ExecutionJournal::new(
        Arc::new(MemoryBackend::new()),
        "proof-close",
        Limits::default(),
    );
    let opened = journal.open(pins(), None).expect("journal");
    (journal, opened.head)
}

pub(crate) fn recorded_proofs(journal: &ExecutionJournal<MemoryBackend>) -> Vec<(String, bool)> {
    journal
        .rebuild()
        .expect("rebuild")
        .records
        .into_iter()
        .filter_map(|record| match record {
            ExecutionRecord::ProofResult {
                proof,
                passed,
                evidence,
            } => {
                assert!(evidence.is_none());
                Some((proof, passed))
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn proof_line(fence: &ContentId, claim_id: &str, leaf: &str) -> String {
    let id = fence
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("revision/{id};claim/{claim_id};leaf/{leaf}")
}

pub(crate) fn equality(name: &str, bytes: &[u8]) -> ProofLeaf {
    ProofLeaf::ArtifactEquality {
        name: name.to_owned(),
        left: bytes.to_vec(),
        right: bytes.to_vec(),
    }
}

pub(crate) fn failed_equality(name: &str, left: &[u8], right: &[u8]) -> ProofLeaf {
    ProofLeaf::ArtifactEquality {
        name: name.to_owned(),
        left: left.to_vec(),
        right: right.to_vec(),
    }
}

pub(crate) fn command_leaf(name: &str) -> ProofLeaf {
    ProofLeaf::command(CommandProof {
        name: name.to_owned(),
        effect_id: format!("effect-{name}"),
        program: "tool:harmless".to_owned(),
        argv: vec!["--literal".to_owned()],
        working_directory: "/source".to_owned(),
        environment: BTreeMap::new(),
        allowed_environment_keys: BTreeSet::new(),
        source_mount: "checkout:exact-head".to_owned(),
        scratch_mount: Some("scratch:proof".to_owned()),
        source_read_only: true,
        limits: SandboxLimits {
            cpu_seconds: 1,
            memory_bytes: 32 * 1024 * 1024,
            wall_time_ms: 100,
            process_count: 2,
            file_count: 2,
            file_bytes: 1024,
            output_bytes: 1024,
            stdin_bytes: 1,
        },
        expected: StructuredExpectation {
            stdout: ProofOutputId::sha256(b""),
            exit_code: 0,
        },
    })
}

pub(crate) fn deck_leaf(name: &str) -> ProofLeaf {
    ProofLeaf::SourceDeckPredicate {
        name: name.to_owned(),
        actual_deck: "deck/actual".to_owned(),
        expected_deck: "deck/actual".to_owned(),
        required_claims: BTreeSet::from(["claim/src".to_owned()]),
        present_claims: BTreeSet::from(["claim/src".to_owned()]),
    }
}

pub(crate) fn catalog(leaves: impl IntoIterator<Item = ProofLeaf>) -> ProofCatalog {
    ProofCatalog::new(leaves).expect("catalog")
}

pub(crate) fn attachment(claim_id: &str, proof: &str) -> super::ProofAttachment {
    super::ProofAttachment {
        claim: claim_id.to_owned(),
        proof: proof.to_owned(),
    }
}

pub(crate) fn submit(
    revision: &mut Revision,
    catalog: &ProofCatalog,
    journal: &ExecutionJournal<MemoryBackend>,
    head: &JournalHead,
    attachments: &[super::ProofAttachment],
) -> Result<AttachStep, AttachError> {
    let fence = revision.id.clone();
    super::attach_proofs(revision, catalog, &fence, journal, head, attachments)
}

pub(crate) fn closure_of(revision: &Revision, claim_id: &str) -> Vec<u8> {
    let state = &revision.claims[claim_id];
    super::close_body::input_closure(&state.inputs, &revision.images).expect("known inputs")
}

pub(crate) fn canon(path: &str, bytes: &[u8]) -> Vec<u8> {
    let inputs = BTreeSet::from([path.to_owned()]);
    let images = BTreeMap::from([(path.to_owned(), bytes.to_vec())]);
    super::close_body::input_closure(&inputs, &images).expect("canon")
}

pub(crate) fn reopen(revision: &mut Revision, path: &str, bytes: &[u8]) -> BTreeSet<String> {
    let step = super::propose(revision, complete(path, bytes, 100)).expect("propose");
    match step {
        Step::Invalidated { affected, .. } => affected,
        other => panic!("expected invalidation, got {other:?}"),
    }
}

pub(crate) fn commit_both(revision: &mut Revision) -> BTreeSet<String> {
    let step = super::propose(
        revision,
        Proposal {
            author: "author".to_owned(),
            checker: "checker".to_owned(),
            fence: Fence::Commit,
            body: ProposalBody::Complete {
                id: "proposal/both".to_owned(),
                facts: BTreeMap::from([
                    ("fact/alpha".to_owned(), b"alpha-1".to_vec()),
                    ("fact/beta".to_owned(), b"beta-1".to_vec()),
                ]),
                limit: 100,
            },
        },
    )
    .expect("propose");
    match step {
        Step::Invalidated { affected, .. } => affected,
        other => panic!("expected invalidation, got {other:?}"),
    }
}

pub(crate) fn revision_with(path: &str, bytes: &[u8], claim_id: &str) -> Revision {
    let seal = b"seal-v1".to_vec();
    let seal_id = super::seal_id(&seal).expect("seal");
    let mut images = BTreeMap::new();
    images.insert(path.to_owned(), bytes.to_vec());
    open(super::OpenRequest {
        seal,
        seal_id,
        images,
        claims: vec![claim(claim_id, &[path])],
    })
    .expect("revision")
}

fn pins() -> ExecutionPins {
    ExecutionPins {
        conduct: "conduct".to_owned(),
        policy: "policy".to_owned(),
        source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [1; 32]),
        model_pick: "model".to_owned(),
        runner_generation: "runner".to_owned(),
    }
}
