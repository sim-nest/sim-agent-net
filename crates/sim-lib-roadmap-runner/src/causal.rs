// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{ContentId, Datum, Symbol};

use crate::{ExecutionJournalError, SemanticChange};

pub(crate) fn screen_changes(
    facts: &BTreeMap<String, ContentId>,
    changes: &[SemanticChange],
    sequence: u64,
) -> Result<bool, ExecutionJournalError> {
    let mut keys = BTreeSet::new();
    let mut changed = false;
    for change in changes {
        if change.key.is_empty() || !keys.insert(change.key.clone()) {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "semantic delta key is empty or repeated",
            });
        }
        if facts.get(&change.key) != change.before.as_ref() {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "semantic delta before value disagrees",
            });
        }
        if change.before != change.after {
            changed = true;
        }
    }
    Ok(changed)
}

pub(crate) fn apply_changes(
    facts: &mut BTreeMap<String, ContentId>,
    changes: &[SemanticChange],
    sequence: u64,
) -> Result<bool, ExecutionJournalError> {
    let changed = screen_changes(facts, changes, sequence)?;
    if changed {
        for change in changes {
            if change.before != change.after {
                match &change.after {
                    Some(after) => {
                        facts.insert(change.key.clone(), after.clone());
                    }
                    None => {
                        facts.remove(&change.key);
                    }
                }
            }
        }
    }
    Ok(changed)
}

pub(crate) fn content_id_datum(id: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", "content-id-v1"),
        fields: vec![
            (
                Symbol::new("algorithm"),
                Datum::Symbol(id.algorithm.clone()),
            ),
            (Symbol::new("digest"), Datum::Bytes(id.bytes.to_vec())),
        ],
    }
}

pub(crate) fn content_id_from_datum(value: &Datum) -> Result<ContentId, ExecutionJournalError> {
    let Datum::Node { tag, fields } = value else {
        return Err(ExecutionJournalError::Semantic("content id shape"));
    };
    if *tag != Symbol::qualified("roadmap-execution", "content-id-v1") || fields.len() != 2 {
        return Err(ExecutionJournalError::Semantic("content id shape"));
    }
    let algorithm = fields
        .iter()
        .find_map(|(name, value)| (*name == Symbol::new("algorithm")).then_some(value));
    let digest = fields
        .iter()
        .find_map(|(name, value)| (*name == Symbol::new("digest")).then_some(value));
    let (Some(Datum::Symbol(algorithm)), Some(Datum::Bytes(digest))) = (algorithm, digest) else {
        return Err(ExecutionJournalError::Semantic("content id fields"));
    };
    let digest: [u8; 32] = digest
        .as_slice()
        .try_into()
        .map_err(|_| ExecutionJournalError::Semantic("content id digest"))?;
    Ok(ContentId::from_bytes(algorithm.clone(), digest))
}
