use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{ContentId, Datum, Symbol};

use crate::{CausalState, ExecutionJournalError, SemanticChange};

pub(crate) fn apply_changes(
    facts: &mut BTreeMap<String, ContentId>,
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
    Ok(changed)
}

pub(crate) fn evidence_set_datum(evidence: &BTreeSet<ContentId>) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", "evidence-set-v1"),
        fields: vec![(
            Symbol::new("members"),
            Datum::Set(evidence.iter().map(content_id_datum).collect()),
        )],
    }
}

pub(crate) fn parse_evidence_set(
    value: &Datum,
) -> Result<BTreeSet<ContentId>, ExecutionJournalError> {
    let Datum::Node { tag, fields } = value else {
        return Err(ExecutionJournalError::Semantic("evidence set shape"));
    };
    if *tag != Symbol::qualified("roadmap-execution", "evidence-set-v1") || fields.len() != 1 {
        return Err(ExecutionJournalError::Semantic("evidence set shape"));
    }
    let (name, Datum::Set(members)) = &fields[0] else {
        return Err(ExecutionJournalError::Semantic("evidence set members"));
    };
    if *name != Symbol::new("members") {
        return Err(ExecutionJournalError::Semantic("evidence set members"));
    }
    members.iter().map(content_id_from_datum).collect()
}

pub(crate) fn causal_state_datum(state: &CausalState, evidence_set: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("roadmap-execution", "snapshot-v1"),
        fields: vec![
            (
                Symbol::new("facts"),
                Datum::Map(
                    state
                        .facts
                        .iter()
                        .map(|(key, value)| (Datum::String(key.clone()), content_id_datum(value)))
                        .collect(),
                ),
            ),
            (Symbol::new("evidence-set"), content_id_datum(evidence_set)),
        ],
    }
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
