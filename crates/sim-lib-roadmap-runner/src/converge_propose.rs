// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Proposal judgment for an accepted revision.

use std::collections::{BTreeMap, BTreeSet};

use crate::schedule::check_id;
use crate::{MAX_MUTATION_FILES, PortableImage, SealedMutationPlan, StructuralEdit};

use super::{
    ConvergeError, Descent, Fence, MalformedMark, POSTIMAGE_MODE, Proposal, ProposalBody, Revision,
    SOURCE_SEAL, Step, changed_facts, image_id, mutation_reason, revision_id, seal_id,
};

/// Judge one proposal. Only [`Fence::Commit`] from a different checker can commit.
pub fn propose(revision: &mut Revision, proposal: Proposal) -> Result<Step, ConvergeError> {
    let id = match &proposal.body {
        ProposalBody::Complete { id, .. }
        | ProposalBody::Incomplete { id, .. }
        | ProposalBody::Malformed { id, .. } => id.clone(),
    };
    if check_id(&id).is_err() {
        return Ok(refused(revision, "invalid-id"));
    }
    match proposal.body {
        ProposalBody::Incomplete { remaining, .. } => incomplete(revision, &remaining),
        ProposalBody::Malformed {
            reason,
            unresolved,
            retry,
            ..
        } => Ok(malformed(revision, &id, &reason, unresolved, retry)),
        ProposalBody::Complete { facts, limit, .. } => complete(
            revision,
            &proposal.author,
            &proposal.checker,
            proposal.fence,
            facts,
            limit,
        ),
    }
}

fn complete(
    revision: &mut Revision,
    author: &str,
    checker: &str,
    fence: Fence,
    facts: BTreeMap<String, Vec<u8>>,
    limit: usize,
) -> Result<Step, ConvergeError> {
    if author.is_empty() || checker.is_empty() {
        return Ok(refused(revision, "empty-identity"));
    }
    if author == checker {
        return Ok(refused(revision, "self-approval"));
    }
    if revision.uncertain {
        return Ok(refused(revision, "uncertain"));
    }
    if facts.len() > MAX_MUTATION_FILES {
        return Ok(refused(revision, "file-ceiling"));
    }
    let bytes = facts
        .values()
        .try_fold(0usize, |sum, fact| sum.checked_add(fact.len()));
    if bytes.is_none_or(|sum| sum > limit) {
        return Ok(refused(revision, "byte-ceiling"));
    }
    if facts.keys().any(|path| !revision.images.contains_key(path)) {
        return Ok(refused(revision, "unknown-fact"));
    }
    if facts.get(SOURCE_SEAL).is_some_and(|bytes| bytes.is_empty()) {
        return Ok(refused(revision, "empty-seal"));
    }
    let differing: BTreeMap<String, Vec<u8>> = facts
        .iter()
        .filter(|(path, post)| revision.images.get(path.as_str()) != Some(post))
        .map(|(path, post)| (path.clone(), post.clone()))
        .collect();
    if differing.is_empty() {
        return Ok(Step::Oscillation {
            revision: revision.id.clone(),
            repeated: image_id(&revision.images)?,
            delta: BTreeSet::new(),
        });
    }
    let edits = differing
        .into_iter()
        .map(|(path, post)| {
            let pre = revision.images[&path].clone();
            StructuralEdit {
                path,
                preimage: PortableImage::file(pre, POSTIMAGE_MODE),
                postimage: PortableImage::file(post, POSTIMAGE_MODE),
            }
        })
        .collect();
    let plan = match SealedMutationPlan::seal(edits) {
        Ok(plan) => plan,
        Err(error) => return Ok(refused(revision, mutation_reason(&error))),
    };
    if fence == Fence::BeforePerform {
        return Ok(Step::Held {
            revision: revision.id.clone(),
        });
    }
    let mut candidate = revision.images.clone();
    for entry in &plan.entries {
        let Some(bytes) = entry.postimage.bytes.clone() else {
            return Ok(refused(revision, "invalid-image"));
        };
        candidate.insert(entry.path.clone(), bytes);
    }
    let Some(seal_bytes) = candidate.get(SOURCE_SEAL) else {
        return Ok(refused(revision, "missing-seal"));
    };
    if seal_bytes.is_empty() {
        return Ok(refused(revision, "empty-seal"));
    }
    if fence == Fence::BeforeCommit {
        revision.uncertain = true;
        return Ok(Step::Held {
            revision: revision.id.clone(),
        });
    }
    let candidate_id = image_id(&candidate)?;
    if revision.seen_images.contains(&candidate_id) {
        return Ok(Step::Oscillation {
            revision: revision.id.clone(),
            repeated: candidate_id,
            delta: changed_facts(&revision.images, &candidate),
        });
    }
    let new_seal = seal_id(seal_bytes)?;
    let changed = changed_facts(&revision.images, &candidate);
    let mut claims = revision.claims.clone();
    let mut affected = BTreeSet::new();
    for (id, claim) in &mut claims {
        if claim.inputs.iter().any(|input| changed.contains(input)) {
            claim.closed = false;
            claim.digest = None;
            affected.insert(id.clone());
        }
    }
    let new_id = revision_id(&new_seal, &candidate_id, &claims)?;
    let closed = claims
        .iter()
        .filter(|(_, claim)| claim.closed)
        .map(|(id, _)| id.clone())
        .collect();
    revision.seal = new_seal;
    revision.images = candidate;
    revision.claims = claims;
    revision.seen_images.insert(candidate_id);
    revision.seen_revisions.insert(new_id.clone());
    revision.id = new_id.clone();
    Ok(Step::Invalidated {
        revision: new_id,
        plan: plan.id,
        affected,
        closed,
    })
}

fn incomplete(revision: &Revision, remaining: &[String]) -> Result<Step, ConvergeError> {
    if remaining.is_empty() {
        return Ok(refused(revision, "empty-needs"));
    }
    let mut seen = BTreeSet::new();
    for need in remaining {
        if check_id(need).is_err() {
            return Ok(refused(revision, "invalid-id"));
        }
        if !seen.insert(need.clone()) {
            return Ok(refused(revision, "duplicate-need"));
        }
    }
    Ok(Step::Incomplete {
        revision: revision.id.clone(),
        remaining: remaining.to_vec(),
    })
}

fn malformed(
    revision: &mut Revision,
    id: &str,
    reason: &str,
    unresolved: u64,
    retry: Option<Descent>,
) -> Step {
    if reason.is_empty() {
        return refused(revision, "empty-reason");
    }
    match (revision.malformed.get(id).cloned(), retry) {
        (Some(mark), _) if mark.retried => refused(revision, "retry-bound"),
        (None, None) => {
            if unresolved == 0 {
                return refused(revision, "empty-unknowns");
            }
            revision.malformed.insert(
                id.to_owned(),
                MalformedMark {
                    unresolved,
                    retried: false,
                },
            );
            Step::Malformed {
                revision: revision.id.clone(),
                reason: reason.to_owned(),
            }
        }
        (None, Some(_)) | (Some(_), None) => refused(revision, "descent"),
        (Some(mark), Some(descent)) => {
            let shrunk = descent.before == mark.unresolved
                && descent.after < descent.before
                && unresolved == descent.after;
            if !shrunk {
                return refused(revision, "descent");
            }
            revision.malformed.insert(
                id.to_owned(),
                MalformedMark {
                    unresolved: descent.after,
                    retried: true,
                },
            );
            Step::Malformed {
                revision: revision.id.clone(),
                reason: reason.to_owned(),
            }
        }
    }
}

fn refused(revision: &Revision, reason: &'static str) -> Step {
    Step::Refused {
        revision: revision.id.clone(),
        reason,
    }
}
