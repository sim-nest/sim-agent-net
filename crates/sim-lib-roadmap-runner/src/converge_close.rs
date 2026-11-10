// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Catalog-backed reclosure of claims opened by a committed postimage.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_lib_journal::{JournalBackend, JournalHead};
use thiserror::Error;

use crate::{ExecutionJournal, ExecutionJournalError, ExecutionRecord, ProofCatalog, ProofLeaf};

use super::{ConvergeError, Revision, claim_digest, revision_id};

/// One catalog leaf offered as the check of one open claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofAttachment {
    /// Claim id in the accepted revision.
    pub claim: String,
    /// Catalog leaf that carried the checked input bytes.
    pub proof: String,
}

/// Result of attaching proof results to the fenced revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachStep {
    /// At least one open claim closed. Seen images, malformed marks, and
    /// uncertainty stay. The successor revision id is recorded.
    Closed {
        /// Successor revision id.
        revision: ContentId,
        /// Claims this call closed.
        closed: BTreeSet<String>,
        /// Claims that remain open.
        open: BTreeSet<String>,
        /// Why a requested claim stayed open.
        reasons: BTreeMap<String, &'static str>,
        /// Journal head after the proof records.
        head: JournalHead,
    },
    /// No claim closed. The accepted revision is unchanged.
    Open {
        /// Accepted revision id.
        revision: ContentId,
        /// Why each requested claim stayed open.
        reasons: BTreeMap<String, &'static str>,
        /// Journal head after the proof records.
        head: JournalHead,
    },
    /// The call was rejected. Nothing was journaled.
    Refused {
        /// Accepted revision id.
        revision: ContentId,
        /// Stable reason.
        reason: &'static str,
    },
}

/// Journal or identity failure while attaching a proof. A refusal is an [`AttachStep`].
#[derive(Debug, Error)]
pub enum AttachError {
    /// The successor revision id could not be formed.
    #[error(transparent)]
    Converge(#[from] ConvergeError),
    /// The proof record was not admitted.
    #[error(transparent)]
    Journal(#[from] ExecutionJournalError),
}

/// Close claims whose current input bytes were checked by a catalog leaf.
///
/// `fence` must equal the revision id derived from the accepted seal bytes,
/// image, and claims. Writing [`Revision::id`] or [`Revision::seal`] does not
/// move that fence. An artifact-equality leaf closes a claim only when both
/// sides equal the canonical bytes of that claim's current inputs. A
/// source-deck or command leaf does not check those bytes and is not
/// launched. Failed and stale checks stay open. Seen images, seen revisions,
/// malformed marks, and uncertainty are kept. Each classified row is
/// journaled before the revision changes. A refused call journals nothing.
pub fn attach_proofs<B: JournalBackend>(
    revision: &mut Revision,
    catalog: &ProofCatalog,
    fence: &ContentId,
    journal: &ExecutionJournal<B>,
    head: &JournalHead,
    attachments: &[ProofAttachment],
) -> Result<AttachStep, AttachError> {
    let accepted = accepted_id(revision)?;
    if fence != &accepted {
        return Ok(refused(&accepted, "stale-revision"));
    }
    if revision.uncertain {
        return Ok(refused(&accepted, "uncertain"));
    }
    if attachments.is_empty() {
        return Ok(refused(&accepted, "empty-attachment"));
    }
    let rows = match classify(revision, catalog, attachments) {
        Ok(rows) => rows,
        Err(reason) => return Ok(refused(&accepted, reason)),
    };
    let successor = successor(revision, &rows)?;
    let mut head = head.clone();
    // A later refusal leaves earlier proof records. The revision changes only
    // after every row in this call has been admitted.
    for row in &rows {
        head = journal.append(
            Some(&head),
            ExecutionRecord::ProofResult {
                proof: proof_record(&accepted, &row.claim, &row.proof),
                passed: row.closes,
                evidence: None,
            },
            Vec::new(),
        )?;
    }
    let Some(successor) = successor else {
        return Ok(AttachStep::Open {
            revision: accepted,
            reasons: reasons(&rows),
            head,
        });
    };
    revision.seal = successor.seal;
    revision.claims = successor.claims;
    revision.seen_revisions.insert(successor.id.clone());
    revision.id = successor.id.clone();
    Ok(AttachStep::Closed {
        revision: successor.id,
        closed: successor.closed,
        open: successor.open,
        reasons: reasons(&rows),
        head,
    })
}

/// Identity of the private image and claims. The public id and seal fields
/// are not inputs.
pub(crate) fn accepted_id(revision: &Revision) -> Result<ContentId, ConvergeError> {
    let seal = bound_seal(revision)?;
    let image = super::image_id(&revision.images)?;
    super::revision_id(&seal, &image, &revision.claims)
}

fn bound_seal(revision: &Revision) -> Result<ContentId, ConvergeError> {
    let Some(bytes) = revision.images.get(super::SOURCE_SEAL) else {
        return Err(ConvergeError::EmptySeal);
    };
    if bytes.is_empty() {
        return Err(ConvergeError::EmptySeal);
    }
    super::seal_id(bytes)
}

/// Canonical bytes of one claim's current inputs. Map order does not matter.
pub(crate) fn input_closure(
    inputs: &BTreeSet<String>,
    images: &BTreeMap<String, Vec<u8>>,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for path in inputs {
        let bytes = images.get(path)?;
        push_chunk(&mut out, path.as_bytes());
        push_chunk(&mut out, bytes);
    }
    Some(out)
}

struct Row {
    claim: String,
    proof: String,
    closes: bool,
    reason: Option<&'static str>,
}

struct Successor {
    id: ContentId,
    seal: ContentId,
    claims: BTreeMap<String, super::ClaimState>,
    closed: BTreeSet<String>,
    open: BTreeSet<String>,
}

fn classify(
    revision: &Revision,
    catalog: &ProofCatalog,
    attachments: &[ProofAttachment],
) -> Result<Vec<Row>, &'static str> {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        if attachment.claim.is_empty() || attachment.proof.is_empty() {
            return Err("empty-attachment");
        }
        if !seen.insert(attachment.claim.clone()) {
            return Err("duplicate-claim");
        }
        let Some(claim) = revision.claims.get(&attachment.claim) else {
            return Err("unknown-claim");
        };
        if claim.closed {
            return Err("already-closed");
        }
        let leaf = catalog
            .leaf(&attachment.proof)
            .map_err(|_| "unknown-proof")?;
        let Some(closure) = input_closure(&claim.inputs, &revision.images) else {
            rows.push(Row {
                claim: attachment.claim.clone(),
                proof: attachment.proof.clone(),
                closes: false,
                reason: Some("unknown-input"),
            });
            continue;
        };
        let (closes, reason) = verdict(leaf, &closure);
        rows.push(Row {
            claim: attachment.claim.clone(),
            proof: attachment.proof.clone(),
            closes,
            reason,
        });
    }
    rows.sort_by(|left, right| left.claim.cmp(&right.claim));
    Ok(rows)
}

fn successor(revision: &Revision, rows: &[Row]) -> Result<Option<Successor>, ConvergeError> {
    if !rows.iter().any(|row| row.closes) {
        return Ok(None);
    }
    let mut claims = revision.claims.clone();
    let mut closed = BTreeSet::new();
    for row in rows {
        if !row.closes {
            continue;
        }
        let state = claims
            .get_mut(&row.claim)
            .expect("classified claim is present");
        state.digest = Some(claim_digest(&row.claim, &state.inputs, &revision.images)?);
        state.closed = true;
        closed.insert(row.claim.clone());
    }
    let seal = bound_seal(revision)?;
    let image = super::image_id(&revision.images)?;
    let id = revision_id(&seal, &image, &claims)?;
    let open = claims
        .iter()
        .filter(|(_, claim)| !claim.closed)
        .map(|(id, _)| id.clone())
        .collect();
    Ok(Some(Successor {
        id,
        seal,
        claims,
        closed,
        open,
    }))
}

fn verdict(leaf: &ProofLeaf, closure: &[u8]) -> (bool, Option<&'static str>) {
    match leaf {
        ProofLeaf::ArtifactEquality { left, right, .. } => {
            if left.as_slice() == closure && right.as_slice() == closure {
                (true, None)
            } else if left == right {
                (false, Some("stale-inputs"))
            } else {
                (false, Some("proof-failed"))
            }
        }
        ProofLeaf::Command(_) | ProofLeaf::SourceDeckPredicate { .. } => {
            (false, Some("unchecked-inputs"))
        }
    }
}

fn reasons(rows: &[Row]) -> BTreeMap<String, &'static str> {
    rows.iter()
        .filter_map(|row| row.reason.map(|reason| (row.claim.clone(), reason)))
        .collect()
}

fn proof_record(fence: &ContentId, claim: &str, proof: &str) -> String {
    format!("revision/{};claim/{claim};leaf/{proof}", hex(fence))
}

fn refused(revision: &ContentId, reason: &'static str) -> AttachStep {
    AttachStep::Refused {
        revision: revision.clone(),
        reason,
    }
}

fn hex(id: &ContentId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(id.bytes.len() * 2);
    for byte in id.bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn push_chunk(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
