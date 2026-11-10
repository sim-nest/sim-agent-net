// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Accepted revisions over disposable postimages.
//!
//! A proposal is data. The accepted images change only when a different
//! checker commits a sealed plan whose bytes differ and whose image has not
//! been seen. Identical bytes are a repeated image and are not sealed. A
//! changed input opens only the claims that read it. Those claims close again
//! only through [`attach_proofs`], when a catalog leaf checks their current
//! input bytes. A stop before commit, an incomplete body, a malformed body,
//! or a repeated image leaves the accepted revision in place. Seen images,
//! seen revisions, and malformed history stay across that reclosure.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{ContentId, Datum, Symbol};
use thiserror::Error;

use crate::mutation::check_accepted_files;
use crate::schedule::check_id;
use crate::{
    FailureFrontier, MutationError, ProofOutcome, ProofUnit, ScheduleError, failure_frontier,
};

#[path = "converge_close.rs"]
mod close_body;
#[path = "converge_propose.rs"]
mod propose_body;

pub use close_body::{AttachError, AttachStep, ProofAttachment, attach_proofs};
pub use propose_body::propose;

/// Fact that stores the source-seal bytes inside an accepted image.
pub const SOURCE_SEAL: &str = "source-seal";

/// Portable mode used for every sealed postimage in this revision.
pub const POSTIMAGE_MODE: u32 = 0o644;

/// Why opening a revision refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConvergeError {
    /// The seal bytes are empty.
    #[error("empty source seal")]
    EmptySeal,
    /// The supplied seal id is not the content id of the seal bytes.
    #[error("source seal does not match its content id")]
    SealMismatch,
    /// A claim names an input the image does not contain.
    #[error("claim input is not in the image")]
    UnknownInput,
    /// Two claims share one id.
    #[error("duplicate claim")]
    DuplicateClaim,
    /// An id cannot be stored.
    #[error("invalid id")]
    InvalidId,
    /// Sealing the image failed.
    #[error("mutation refused: {0}")]
    Mutation(&'static str),
    /// A content id could not be formed.
    #[error("identity: {0}")]
    Identity(String),
}

/// One claim supplied when a revision is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenClaim {
    /// Stable claim id.
    pub id: String,
    /// Fact ids this claim reads.
    pub inputs: BTreeSet<String>,
}

/// Caller-supplied seal, images, and claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    /// Exact source-seal bytes.
    pub seal: Vec<u8>,
    /// Content id of `seal`, from [`seal_id`].
    pub seal_id: ContentId,
    /// Fact path to exact bytes. `source-seal` is filled from `seal` when absent.
    pub images: BTreeMap<String, Vec<u8>>,
    /// Claims that start closed over those images.
    pub claims: Vec<OpenClaim>,
}

/// Closed or open claim over a fixed input set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimState {
    /// Fact ids this claim reads.
    pub inputs: BTreeSet<String>,
    /// Closed claims still match the image they were sealed against.
    pub closed: bool,
    /// Content id of the claim id, input paths, and input bytes.
    /// Cleared when the claim opens.
    pub digest: Option<ContentId>,
}

/// Accepted revision. Images change only inside [`propose`].
/// Claims open inside [`propose`] and close inside [`attach_proofs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Identity of the seal, the image, and each claim's id, closure, digest, and inputs.
    pub id: ContentId,
    /// Content id of the current source-seal bytes.
    pub seal: ContentId,
    /// Accepted fact bytes, including `source-seal`.
    images: BTreeMap<String, Vec<u8>>,
    /// Claim records keyed by claim id.
    claims: BTreeMap<String, ClaimState>,
    /// Image identities already accepted or opened.
    seen_images: BTreeSet<ContentId>,
    /// Revision identities already accepted or opened.
    seen_revisions: BTreeSet<ContentId>,
    /// Set when a perform stops before commit. Further commits stay refused.
    uncertain: bool,
    malformed: BTreeMap<String, MalformedMark>,
}

impl Revision {
    /// Accepted fact bytes, including the source seal.
    pub fn images(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.images
    }

    /// Claim records keyed by claim id.
    pub fn claims(&self) -> &BTreeMap<String, ClaimState> {
        &self.claims
    }

    /// Whether a perform stopped before commit.
    pub fn is_uncertain(&self) -> bool {
        self.uncertain
    }

    /// Whether an accepted or opened image id is already recorded.
    pub(crate) fn has_seen(&self, image: &ContentId) -> bool {
        self.seen_images.contains(image)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MalformedMark {
    unresolved: u64,
    retried: bool,
}

/// Certificate that a malformed retry shrank its unresolved measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descent {
    /// Unresolved measure recorded for this proposal id.
    pub before: u64,
    /// Strictly smaller measure for the single retry.
    pub after: u64,
}

/// Where a modeled perform stops.
///
/// [`Fence::Commit`] is the only fence that can replace the accepted revision.
/// Earlier fences keep that revision. A stop before commit is uncertain and
/// does not authorize another commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fence {
    /// Stop before the candidate image is built.
    BeforePerform,
    /// Stop after the candidate exists and before it is accepted.
    BeforeCommit,
    /// Commit when the checker, the plan, and the image are legal.
    Commit,
}

/// Hostile proposal body. Bytes are data and carry no effect capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalBody {
    /// Full postimage bytes for named facts.
    Complete {
        /// Proposal id.
        id: String,
        /// Fact path to replacement bytes.
        facts: BTreeMap<String, Vec<u8>>,
        /// Maximum postimage byte sum. A larger body is refused whole.
        limit: usize,
    },
    /// Checked partial evidence with typed needs still open.
    Incomplete {
        /// Proposal id.
        id: String,
        /// Needs that remain. Empty is not an honest incomplete body.
        remaining: Vec<String>,
    },
    /// Opaque failure addressed by proposal id and never applied.
    Malformed {
        /// Proposal id.
        id: String,
        /// Non-empty reason.
        reason: String,
        /// Unresolved measure. The first observation must be non-zero.
        unresolved: u64,
        /// Present only on the single descent-certified retry.
        retry: Option<Descent>,
    },
}

/// One proposal. The author and the checker are identities, not capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Author of the body.
    pub author: String,
    /// Checker that would commit a complete body. Must differ from `author`.
    pub checker: String,
    /// Perform fence.
    pub fence: Fence,
    /// Body to judge.
    pub body: ProposalBody,
}

/// Result of judging one proposal against the accepted revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The perform stopped before commit. The accepted revision is unchanged.
    Held {
        /// Accepted revision id.
        revision: ContentId,
    },
    /// The body was refused and was not applied.
    Refused {
        /// Accepted revision id.
        revision: ContentId,
        /// Stable reason.
        reason: &'static str,
    },
    /// Honest incomplete body. Needs are retained and nothing is committed.
    Incomplete {
        /// Accepted revision id.
        revision: ContentId,
        /// Typed needs from the body.
        remaining: Vec<String>,
    },
    /// Honest malformed body. Nothing is committed.
    Malformed {
        /// Accepted revision id.
        revision: ContentId,
        /// Reason from the body.
        reason: String,
    },
    /// The candidate image was already seen. Nothing is committed.
    Oscillation {
        /// Accepted revision id.
        revision: ContentId,
        /// Image id that repeated.
        repeated: ContentId,
        /// Fact ids whose candidate bytes differ from the accepted image.
        delta: BTreeSet<String>,
    },
    /// The checker committed the plan and opened every affected claim.
    Invalidated {
        /// New accepted revision id.
        revision: ContentId,
        /// Identity of the sealed mutation plan.
        plan: ContentId,
        /// Claims opened because an input byte string changed.
        affected: BTreeSet<String>,
        /// Claims that stayed closed.
        closed: BTreeSet<String>,
    },
}

/// Content id of source-seal bytes.
pub fn seal_id(bytes: &[u8]) -> Result<ContentId, ConvergeError> {
    node("SourceSealV1", Datum::Bytes(bytes.to_vec()))
}

/// Content id of fact bytes. Map order does not change the id.
pub fn image_id(images: &BTreeMap<String, Vec<u8>>) -> Result<ContentId, ConvergeError> {
    let facts = images
        .iter()
        .map(|(path, bytes)| Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "FactV1"),
            fields: vec![
                (Symbol::new("path"), Datum::String(path.clone())),
                (Symbol::new("bytes"), Datum::Bytes(bytes.clone())),
            ],
        })
        .collect();
    node("AcceptedImageV1", Datum::List(facts))
}

/// Open a revision. Every claim starts closed. A bad seal refuses.
pub fn open(request: OpenRequest) -> Result<Revision, ConvergeError> {
    if request.seal.is_empty() {
        return Err(ConvergeError::EmptySeal);
    }
    let seal = seal_id(&request.seal)?;
    if seal != request.seal_id {
        return Err(ConvergeError::SealMismatch);
    }
    let mut images = request.images;
    match images.get(SOURCE_SEAL) {
        Some(bytes) if bytes != &request.seal => return Err(ConvergeError::SealMismatch),
        Some(_) => {}
        None => {
            images.insert(SOURCE_SEAL.to_owned(), request.seal);
        }
    }
    check_accepted_files(&images, POSTIMAGE_MODE)
        .map_err(|error| ConvergeError::Mutation(mutation_reason(&error)))?;
    let mut claims = BTreeMap::new();
    for claim in request.claims {
        check_id(&claim.id).map_err(|_| ConvergeError::InvalidId)?;
        if claims.contains_key(&claim.id) {
            return Err(ConvergeError::DuplicateClaim);
        }
        if claim.inputs.iter().any(|input| !images.contains_key(input)) {
            return Err(ConvergeError::UnknownInput);
        }
        let digest = claim_digest(&claim.id, &claim.inputs, &images)?;
        claims.insert(
            claim.id,
            ClaimState {
                inputs: claim.inputs,
                closed: true,
                digest: Some(digest),
            },
        );
    }
    let image = image_id(&images)?;
    let id = revision_id(&seal, &image, &claims)?;
    let seen_revisions = BTreeSet::from([id.clone()]);
    Ok(Revision {
        id,
        seal,
        images,
        claims,
        seen_images: BTreeSet::from([image]),
        seen_revisions,
        uncertain: false,
        malformed: BTreeMap::new(),
    })
}

/// Failure frontier for the named claims. This does not edit a revision.
pub fn proof_frontier(
    claims: &[String],
    outcomes: &BTreeMap<String, ProofOutcome>,
) -> Result<FailureFrontier, ScheduleError> {
    let units = claims
        .iter()
        .map(|id| ProofUnit {
            id: id.clone(),
            depends_on: Vec::new(),
            capability: "proof".to_owned(),
            invalidation_mass: 1,
            learned_cost: 0,
            critical_path_gain: 0,
            novelty_gain: 0,
            resources: Vec::new(),
        })
        .collect::<Vec<_>>();
    let budget = u32::try_from(claims.len()).unwrap_or(u32::MAX);
    failure_frontier(
        &units,
        outcomes,
        &BTreeSet::from(["proof".to_owned()]),
        &BTreeSet::new(),
        budget,
    )
}

fn mutation_reason(error: &MutationError) -> &'static str {
    match error {
        MutationError::InvalidPath => "invalid-path",
        MutationError::DuplicatePath => "duplicate-path",
        MutationError::ParentChildCollision => "parent-child",
        MutationError::InvalidImage => "invalid-image",
        MutationError::InvalidMode => "invalid-mode",
        MutationError::ByteLimit => "byte-ceiling",
        _ => "mutation",
    }
}

fn changed_facts(
    accepted: &BTreeMap<String, Vec<u8>>,
    candidate: &BTreeMap<String, Vec<u8>>,
) -> BTreeSet<String> {
    accepted
        .keys()
        .chain(candidate.keys())
        .filter(|key| accepted.get(*key) != candidate.get(*key))
        .cloned()
        .collect()
}

fn claim_digest(
    id: &str,
    inputs: &BTreeSet<String>,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<ContentId, ConvergeError> {
    let mut list = vec![Datum::String(id.to_owned())];
    for input in inputs {
        let bytes = images
            .get(input)
            .ok_or(ConvergeError::UnknownInput)?
            .clone();
        list.push(Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "ClaimInputV1"),
            fields: vec![
                (Symbol::new("path"), Datum::String(input.clone())),
                (Symbol::new("bytes"), Datum::Bytes(bytes)),
            ],
        });
    }
    node("ClaimDigestV1", Datum::List(list))
}

fn revision_id(
    seal: &ContentId,
    image: &ContentId,
    claims: &BTreeMap<String, ClaimState>,
) -> Result<ContentId, ConvergeError> {
    let mut list = vec![id_datum(seal), id_datum(image)];
    for (id, claim) in claims {
        let digest = match &claim.digest {
            Some(digest) => id_datum(digest),
            None => Datum::Nil,
        };
        let inputs = Datum::List(claim.inputs.iter().cloned().map(Datum::String).collect());
        list.push(Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "ClaimRecordV1"),
            fields: vec![
                (Symbol::new("id"), Datum::String(id.clone())),
                (Symbol::new("closed"), Datum::Bool(claim.closed)),
                (Symbol::new("digest"), digest),
                (Symbol::new("inputs"), inputs),
            ],
        });
    }
    node("RevisionV1", Datum::List(list))
}

fn id_datum(id: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("core", "ContentId"),
        fields: vec![
            (
                Symbol::new("algorithm"),
                Datum::Symbol(id.algorithm.clone()),
            ),
            (Symbol::new("bytes"), Datum::Bytes(id.bytes.to_vec())),
        ],
    }
}

fn node(name: &str, payload: Datum) -> Result<ContentId, ConvergeError> {
    Datum::Node {
        tag: Symbol::qualified("roadmap-runner", name),
        fields: vec![(Symbol::new("payload"), payload)],
    }
    .content_id()
    .map_err(|error| ConvergeError::Identity(error.to_string()))
}

#[cfg(test)]
#[path = "converge_fixture.rs"]
mod fixture;

#[cfg(test)]
#[path = "converge_tests.rs"]
mod converge_tests;

#[cfg(test)]
#[path = "converge_gate_tests.rs"]
mod converge_gate_tests;

#[cfg(test)]
#[path = "converge_close_fixture.rs"]
mod close_fixture;

#[cfg(test)]
#[path = "converge_close_tests.rs"]
mod converge_close_tests;

#[cfg(test)]
#[path = "converge_close_gate_tests.rs"]
mod converge_close_gate_tests;
