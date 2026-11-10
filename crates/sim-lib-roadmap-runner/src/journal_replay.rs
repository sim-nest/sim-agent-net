// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One chain pass shared by `rebuild` and `rebuild_report`.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_lib_journal::{JournalEntry, JournalError, JournalHead, JournalObject, StoredState};

use crate::causal::apply_changes;
use crate::journal_share::{self, empty_id};
use crate::record_law::{LawCursor, record_refs};
use crate::{
    CausalState, ExecutionJournalError, ExecutionRecord, Limits, RebuiltExecution, ReplayFailure,
    RetentionRoots, VerifiedSnapshot, codec,
};

pub(crate) struct Fold {
    pub(crate) rebuilt: RebuiltExecution,
    pub(crate) facts_root: ContentId,
    pub(crate) law: LawCursor,
}

pub(crate) struct FoldFault {
    pub(crate) last: Option<JournalHead>,
    pub(crate) error: Box<ExecutionJournalError>,
}

pub(crate) fn fold(
    execution_id: &str,
    limits: &Limits,
    state: &StoredState,
) -> Result<Fold, FoldFault> {
    let chain = accept_chain(state);
    if chain.entries.is_empty() {
        return Err(chain
            .fault
            .unwrap_or_else(|| fault(None, ExecutionJournalError::Empty)));
    }
    let checked = decode(execution_id, limits, state, &chain.entries);
    match chain.fault {
        Some(fault) => Err(checked.err().unwrap_or(fault)),
        None => checked,
    }
}

pub(crate) fn report_failure(fault: FoldFault) -> ReplayFailure {
    ReplayFailure {
        last_verified_head: fault.last,
        error: fault.error,
    }
}

struct AcceptedChain {
    entries: Vec<JournalEntry>,
    fault: Option<FoldFault>,
}

fn accept_chain(state: &StoredState) -> AcceptedChain {
    let mut last = None;
    let mut entries = Vec::new();
    for (index, entry) in state.entries.values().enumerate() {
        if let Some(error) = reject_link(state, entry, index as u64, last.as_ref()) {
            return AcceptedChain {
                entries,
                fault: Some(fault(last, error.into())),
            };
        }
        entries.push(entry.clone());
        last = Some(JournalHead {
            sequence: entry.sequence,
            entry: entry.id.clone(),
        });
    }
    let fault = if state.head != last {
        Some(fault(last, JournalError::CorruptState("head").into()))
    } else if entries.is_empty() {
        Some(fault(None, ExecutionJournalError::Empty))
    } else {
        None
    };
    AcceptedChain { entries, fault }
}

fn reject_link(
    state: &StoredState,
    entry: &JournalEntry,
    sequence: u64,
    last: Option<&JournalHead>,
) -> Option<JournalError> {
    if entry.sequence != sequence {
        return Some(JournalError::CorruptState("sequence gap"));
    }
    if entry.previous.as_ref() != last.map(|head| &head.entry) {
        return Some(JournalError::CorruptState("previous id"));
    }
    match entry.canonical_datum().content_id() {
        Ok(id) if id == entry.id => {}
        Ok(_) | Err(_) => return Some(JournalError::CorruptEntry),
    }
    for payload in &entry.payloads {
        let Some(bytes) = state.objects.get(payload) else {
            return Some(JournalError::MissingPayload(payload.clone()));
        };
        let Some(datum) = state.datums.get(payload) else {
            return Some(JournalError::MissingSemanticObject(payload.clone()));
        };
        match JournalObject::from_datum(datum.clone()) {
            Ok(object) if object.id == *payload && object.bytes == *bytes => {}
            Ok(_) => return Some(JournalError::CorruptObject(payload.clone())),
            Err(error) => return Some(error),
        }
    }
    None
}

fn decode(
    execution_id: &str,
    limits: &Limits,
    state: &StoredState,
    accepted: &[JournalEntry],
) -> Result<Fold, FoldFault> {
    let mut records = Vec::new();
    let mut pins = None;
    let mut total = 0usize;
    let mut retention = RetentionRoots::default();
    let mut snapshots = Vec::new();
    let mut law = LawCursor::new();
    for (index, entry) in accepted.iter().enumerate() {
        let record = match decode_entry(
            execution_id,
            state,
            entry,
            &mut law,
            &mut total,
            &mut retention,
        ) {
            Ok(record) => record,
            Err(error) => return Err(law_fault(state, accepted, &records, index, error)),
        };
        if let ExecutionRecord::ExecutionOpened { pins: opened, .. } = &record {
            pins = Some(opened.clone());
        }
        if let ExecutionRecord::Snapshot {
            covers,
            state: state_id,
            evidence_set,
        } = &record
        {
            if let Err(error) = snapshot_shell(state, entry, covers, state_id, evidence_set) {
                return Err(law_fault(state, accepted, &records, index, error));
            }
            snapshots.push(VerifiedSnapshot {
                covers: covers.clone(),
                state: state_id.clone(),
                evidence_set: evidence_set.clone(),
            });
        }
        records.push(record);
    }
    let (causal, facts_root, evidence_root, causal_applies) = confirm(state, &records, accepted)?;
    let last = accepted.last().map(|entry| JournalHead {
        sequence: entry.sequence,
        entry: entry.id.clone(),
    });
    if total > limits.max_execution_bytes {
        return Err(fault(last, ExecutionJournalError::Budget("execution")));
    }
    let opening = &accepted[0];
    Ok(Fold {
        rebuilt: RebuiltExecution {
            execution_id: execution_id.to_owned(),
            pins: pins
                .ok_or_else(|| fault(last.clone(), ExecutionJournalError::ExecutionIdentity))?,
            records,
            opening_head: JournalHead {
                sequence: opening.sequence,
                entry: opening.id.clone(),
            },
            head: last.ok_or_else(|| fault(None, ExecutionJournalError::Empty))?,
            total_bytes: total,
            causal,
            snapshots,
            retention,
            causal_applies,
            evidence_root,
        },
        facts_root,
        law,
    })
}

fn law_fault(
    state: &StoredState,
    accepted: &[JournalEntry],
    records: &[ExecutionRecord],
    index: usize,
    error: ExecutionJournalError,
) -> FoldFault {
    match confirm(state, records, &accepted[..index]) {
        Ok(_) => fault(head_before(accepted, index), error),
        Err(causal) => causal,
    }
}

fn confirm(
    state: &StoredState,
    records: &[ExecutionRecord],
    accepted: &[JournalEntry],
) -> Result<(CausalState, ContentId, ContentId, u64), FoldFault> {
    let boundary = records
        .iter()
        .rposition(|record| matches!(record, ExecutionRecord::Snapshot { .. }));
    let mut causal = CausalState::default();
    let mut evidence_root = empty_id();
    let mut facts_root = empty_id();
    let mut causal_applies = 0u64;
    if let Some(boundary) = boundary {
        cover_prefix(
            state,
            records,
            accepted,
            boundary,
            &mut causal,
            &mut evidence_root,
            &mut facts_root,
        )?;
    }
    let suffix_from = boundary.map_or(0, |index| index + 1);
    for (index, record) in records.iter().enumerate().skip(suffix_from) {
        let ExecutionRecord::SemanticDelta {
            changes,
            evidence_set,
            ..
        } = record
        else {
            continue;
        };
        let added = admit_delta(
            state,
            &mut causal.facts,
            &mut evidence_root,
            changes,
            evidence_set,
            accepted[index].sequence,
        )
        .map_err(|error| fault(head_before(accepted, index), error))?;
        causal.evidence.extend(added);
        causal_applies = causal_applies.saturating_add(1);
    }
    Ok((causal, facts_root, evidence_root, causal_applies))
}

fn cover_prefix(
    state: &StoredState,
    records: &[ExecutionRecord],
    accepted: &[JournalEntry],
    boundary: usize,
    causal: &mut CausalState,
    evidence_root: &mut ContentId,
    facts_root: &mut ContentId,
) -> Result<(), FoldFault> {
    let mut covered_facts = BTreeMapFacts::new();
    let mut covered_evidence = empty_id();
    let mut verified = empty_id();
    let mut since = BTreeMap::new();
    for (index, record) in records.iter().enumerate().take(boundary + 1) {
        match record {
            ExecutionRecord::SemanticDelta {
                changes,
                evidence_set,
                ..
            } => {
                admit_delta(
                    state,
                    &mut covered_facts,
                    &mut covered_evidence,
                    changes,
                    evidence_set,
                    accepted[index].sequence,
                )
                .map_err(|error| fault(head_before(accepted, index), error))?;
                for change in changes {
                    note_since(&mut since, change);
                }
            }
            ExecutionRecord::Snapshot {
                state: state_id,
                evidence_set,
                ..
            } => {
                let root = check_snapshot(
                    state,
                    state_id,
                    evidence_set,
                    &covered_evidence,
                    &verified,
                    &since,
                )
                .map_err(|error| fault(head_before(accepted, index), error))?;
                since.clear();
                verified = root;
                if index == boundary {
                    causal.facts.clone_from(&covered_facts);
                    causal.evidence =
                        journal_share::load_evidence(evidence_set, &state.datums, &state.objects)
                            .map_err(|error| fault(head_before(accepted, index), error))?;
                    *evidence_root = evidence_set.clone();
                    *facts_root = verified.clone();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn admit_delta(
    state: &StoredState,
    facts: &mut BTreeMapFacts,
    evidence: &mut ContentId,
    changes: &[crate::SemanticChange],
    evidence_set: &ContentId,
    sequence: u64,
) -> Result<Vec<ContentId>, ExecutionJournalError> {
    let changed = apply_changes(facts, changes, sequence)?;
    if evidence_set == evidence {
        return if changed {
            Ok(Vec::new())
        } else {
            Err(carry_state(sequence))
        };
    }
    let added =
        journal_share::evidence_additions(evidence, evidence_set, &state.datums, &state.objects)?;
    if !changed && added.is_empty() {
        return Err(carry_state(sequence));
    }
    *evidence = evidence_set.clone();
    Ok(added)
}

fn note_since(since: &mut Since, change: &crate::SemanticChange) {
    let base = match since.get(&change.key) {
        Some((base, _)) => base.clone(),
        None => change.before.clone(),
    };
    if base == change.after {
        since.remove(&change.key);
    } else {
        since.insert(change.key.clone(), (base, change.after.clone()));
    }
}

fn check_snapshot(
    state: &StoredState,
    state_id: &ContentId,
    evidence_set: &ContentId,
    covered_evidence: &ContentId,
    verified: &ContentId,
    since: &Since,
) -> Result<ContentId, ExecutionJournalError> {
    let value = state
        .datums
        .get(state_id)
        .ok_or(ExecutionJournalError::MissingObject)?;
    let (facts_root, evidence_root) = journal_share::open_snapshot(value)?;
    if evidence_set != covered_evidence || &evidence_root != covered_evidence {
        return Err(ExecutionJournalError::Semantic("snapshot covered state"));
    }
    let diff = journal_share::fact_changes(verified, &facts_root, &state.datums, &state.objects)?;
    let net: BTreeMap<_, _> = since
        .iter()
        .map(|(key, (_, now))| (key.clone(), now.clone()))
        .collect();
    if diff != net {
        return Err(ExecutionJournalError::Semantic("snapshot covered state"));
    }
    Ok(facts_root)
}

fn carry_state(sequence: u64) -> ExecutionJournalError {
    ExecutionJournalError::Illegal {
        sequence,
        reason: "carry-state semantic delta",
    }
}

fn decode_entry(
    execution_id: &str,
    state: &StoredState,
    entry: &JournalEntry,
    law: &mut LawCursor,
    total: &mut usize,
    retention: &mut RetentionRoots,
) -> Result<ExecutionRecord, ExecutionJournalError> {
    if entry.payloads.is_empty() || entry.kind.namespace.as_deref() != Some("roadmap-execution") {
        return Err(ExecutionJournalError::Illegal {
            sequence: entry.sequence,
            reason: "foreign entry",
        });
    }
    let value = state
        .datums
        .get(&entry.payloads[0])
        .ok_or(ExecutionJournalError::MissingObject)?;
    let bytes = state
        .objects
        .get(&entry.payloads[0])
        .ok_or(ExecutionJournalError::MissingObject)?;
    charge(total, bytes.len())?;
    let (execution, record) = codec::decode_datum(value)?;
    if execution != execution_id {
        return Err(ExecutionJournalError::ExecutionIdentity);
    }
    let refs = record_refs(&record);
    let payloads: BTreeSet<_> = entry.payloads.iter().skip(1).cloned().collect();
    if refs != payloads {
        return Err(ExecutionJournalError::MissingObject);
    }
    for id in &refs {
        let len = state
            .objects
            .get(id)
            .ok_or(ExecutionJournalError::MissingObject)?
            .len();
        charge(total, len)?;
    }
    law.advance(&record, entry.sequence)?;
    retention.entries.insert(entry.id.clone());
    retention.objects.extend(entry.payloads.iter().cloned());
    Ok(record)
}

fn snapshot_shell(
    state: &StoredState,
    entry: &JournalEntry,
    covers: &ContentId,
    state_id: &ContentId,
    evidence_set: &ContentId,
) -> Result<(), ExecutionJournalError> {
    if entry.previous.as_ref() != Some(covers) {
        return Err(ExecutionJournalError::Semantic("snapshot prefix"));
    }
    let value = state
        .datums
        .get(state_id)
        .ok_or(ExecutionJournalError::MissingObject)?;
    let id = value
        .content_id()
        .map_err(|_| ExecutionJournalError::Semantic("snapshot identity"))?;
    if id != *state_id {
        return Err(ExecutionJournalError::Semantic("snapshot identity"));
    }
    let (_facts, evidence) = journal_share::open_snapshot(value)?;
    if &evidence != evidence_set {
        return Err(ExecutionJournalError::Semantic("snapshot state"));
    }
    Ok(())
}

fn head_before(entries: &[JournalEntry], index: usize) -> Option<JournalHead> {
    index.checked_sub(1).map(|prior| JournalHead {
        sequence: entries[prior].sequence,
        entry: entries[prior].id.clone(),
    })
}

fn fault(last: Option<JournalHead>, error: ExecutionJournalError) -> FoldFault {
    FoldFault {
        last,
        error: Box::new(error),
    }
}

fn charge(total: &mut usize, extra: usize) -> Result<(), ExecutionJournalError> {
    *total = total
        .checked_add(extra)
        .ok_or(ExecutionJournalError::Budget("execution"))?;
    Ok(())
}

type BTreeMapFacts = BTreeMap<String, ContentId>;
type Since = BTreeMap<String, (Option<ContentId>, Option<ContentId>)>;
