// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use crate::{PortableImage, SealedMutationPlan, StructuralEdit};

use super::{
    ConvergeError, Descent, Fence, OpenClaim, OpenRequest, POSTIMAGE_MODE, Proposal, ProposalBody,
    Revision, SOURCE_SEAL, open, seal_id,
};

pub(crate) fn claim(id: &str, inputs: &[&str]) -> OpenClaim {
    OpenClaim {
        id: id.to_owned(),
        inputs: inputs.iter().map(|input| (*input).to_owned()).collect(),
    }
}

pub(crate) fn opened() -> Revision {
    let seal = b"seal-v1".to_vec();
    let seal_id = seal_id(&seal).expect("seal id");
    let mut images = BTreeMap::new();
    images.insert("fact/alpha".to_owned(), b"alpha-0".to_vec());
    images.insert("fact/beta".to_owned(), b"beta-0".to_vec());
    open(OpenRequest {
        seal,
        seal_id,
        images,
        claims: vec![
            claim("claim/alpha", &["fact/alpha"]),
            claim("claim/beta", &["fact/beta"]),
            claim("claim/seal", &[SOURCE_SEAL]),
        ],
    })
    .expect("open")
}

pub(crate) fn opened_without(id: &str, inputs: &[&str]) -> Result<Revision, ConvergeError> {
    let seal = b"seal-v1".to_vec();
    let seal_id = seal_id(&seal).expect("seal id");
    let mut images = BTreeMap::new();
    images.insert("fact/alpha".to_owned(), b"alpha-0".to_vec());
    open(OpenRequest {
        seal,
        seal_id,
        images,
        claims: vec![claim(id, inputs)],
    })
}

pub(crate) fn fact(path: &str, bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut facts = BTreeMap::new();
    facts.insert(path.to_owned(), bytes.to_vec());
    facts
}

pub(crate) fn proposal(author: &str, checker: &str, fence: Fence, body: ProposalBody) -> Proposal {
    Proposal {
        author: author.to_owned(),
        checker: checker.to_owned(),
        fence,
        body,
    }
}

pub(crate) fn complete(path: &str, bytes: &[u8], limit: usize) -> Proposal {
    proposal(
        "author",
        "checker",
        Fence::Commit,
        ProposalBody::Complete {
            id: "proposal/1".to_owned(),
            facts: fact(path, bytes),
            limit,
        },
    )
}

pub(crate) fn complete_fence(path: &str, bytes: &[u8], fence: Fence) -> Proposal {
    proposal(
        "author",
        "checker",
        fence,
        ProposalBody::Complete {
            id: "proposal/1".to_owned(),
            facts: fact(path, bytes),
            limit: 100,
        },
    )
}

pub(crate) fn malformed(reason: &str, unresolved: u64, retry: Option<Descent>) -> Proposal {
    proposal(
        "author",
        "author",
        Fence::Commit,
        ProposalBody::Malformed {
            id: "proposal/bad".to_owned(),
            reason: reason.to_owned(),
            unresolved,
            retry,
        },
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcceptedView {
    alpha: Vec<u8>,
    beta_closed: bool,
    beta_digest: Option<Vec<u8>>,
    seal_closed: bool,
    seal_digest: Option<Vec<u8>>,
}

pub(crate) fn accepted(revision: &Revision) -> AcceptedView {
    AcceptedView {
        alpha: revision.images["fact/alpha"].clone(),
        beta_closed: revision.claims["claim/beta"].closed,
        beta_digest: revision.claims["claim/beta"]
            .digest
            .as_ref()
            .map(|id| id.bytes.to_vec()),
        seal_closed: revision.claims["claim/seal"].closed,
        seal_digest: revision.claims["claim/seal"]
            .digest
            .as_ref()
            .map(|id| id.bytes.to_vec()),
    }
}

pub(crate) fn plan_for(pre: &[u8], post: &[u8]) -> SealedMutationPlan {
    let once = SealedMutationPlan::seal(vec![edit(pre, post)]).expect("seal");
    let twice = SealedMutationPlan::seal(vec![edit(pre, post)]).expect("seal");
    assert_eq!(once.id, twice.id);
    once
}

fn edit(pre: &[u8], post: &[u8]) -> StructuralEdit {
    StructuralEdit {
        path: "fact/alpha".to_owned(),
        preimage: PortableImage::file(pre.to_vec(), POSTIMAGE_MODE),
        postimage: PortableImage::file(post.to_vec(), POSTIMAGE_MODE),
    }
}
