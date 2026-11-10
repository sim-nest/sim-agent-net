// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{collections::BTreeSet, sync::Arc};

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::{
    JournalBackend, JournalEntry, JournalError, JournalHead, JournalObject, MemoryBackend,
    StoredState,
};

use crate::journal_contract_tests::{content, pins};
use crate::{
    DeltaAppend, ExecutionJournal, ExecutionJournalError, ExecutionRecord, Limits, SemanticChange,
};

#[test]
fn prefix_carry_state_fails_before_a_matching_snapshot() {
    let journal = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "carry", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let fact = content(4);
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(fact.clone()),
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    let empty = crate::journal_share::empty_id();
    let empty_object = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            crate::journal_share::empty_datum(),
            "empty evidence",
        )
        .unwrap();
    let carry = journal
        .append(
            Some(&delta),
            ExecutionRecord::SemanticDelta {
                cause: "carry".into(),
                changes: vec![SemanticChange {
                    key: "answer".into(),
                    before: Some(fact.clone()),
                    after: Some(fact),
                }],
                evidence_set: empty,
            },
            vec![empty_object],
        )
        .unwrap();
    journal.snapshot(&carry).unwrap();
    let report = journal.rebuild_report().unwrap_err();
    assert!(
        matches!(
            report.error.as_ref(),
            ExecutionJournalError::Illegal {
                reason: "carry-state semantic delta",
                ..
            }
        ),
        "error={:?} head={:?}",
        report.error,
        report.last_verified_head
    );
    assert_eq!(report.last_verified_head.as_ref(), Some(&delta));
}

#[test]
fn prefix_evidence_drop_fails_at_the_delta() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "drop", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let fact = content(4);
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(fact.clone()),
            }],
            BTreeSet::from([content(9)]),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    let empty = crate::journal_share::empty_id();
    let empty_object = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            crate::journal_share::empty_datum(),
            "empty evidence",
        )
        .unwrap();
    let dropped = journal
        .append(
            Some(&delta),
            ExecutionRecord::SemanticDelta {
                cause: "drop-evidence".into(),
                changes: vec![SemanticChange {
                    key: "answer".into(),
                    before: Some(fact),
                    after: Some(content(4)),
                }],
                evidence_set: empty.clone(),
            },
            vec![empty_object],
        )
        .unwrap();
    journal.snapshot(&dropped).unwrap();
    let mut state = backend.read_state().unwrap();
    rewrite_snapshot_evidence(&mut state, "drop", &empty);
    let Err(fault) = crate::journal_replay::fold("drop", &Limits::default(), &state) else {
        panic!("evidence drop was accepted");
    };
    assert!(matches!(
        fault.error.as_ref(),
        ExecutionJournalError::Semantic("evidence set dropped a retained member")
    ));
    assert_eq!(fault.last.as_ref(), Some(&delta));
}

#[test]
fn causal_fault_excludes_the_invalid_delta_from_the_verified_head() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "torn", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let empty_object = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            crate::journal_share::empty_datum(),
            "empty evidence",
        )
        .unwrap();
    let bad = journal
        .append(
            Some(&opened.head),
            ExecutionRecord::SemanticDelta {
                cause: "bad-before".into(),
                changes: vec![SemanticChange {
                    key: "answer".into(),
                    before: Some(content(9)),
                    after: Some(content(4)),
                }],
                evidence_set: crate::journal_share::empty_id(),
            },
            vec![empty_object],
        )
        .unwrap();
    let mut state = backend.read_state().unwrap();
    push_record(
        &mut state,
        "torn",
        ExecutionRecord::EffectReceipt {
            effect_id: "missing".into(),
            outcome: "done".into(),
            output: None,
        },
    );
    let Err(fault) = crate::journal_replay::fold("torn", &Limits::default(), &state) else {
        panic!("invalid delta was accepted");
    };
    assert_eq!(fault.last.as_ref(), Some(&opened.head));
    assert_ne!(fault.last.as_ref(), Some(&bad));
    assert!(matches!(
        fault.error.as_ref(),
        ExecutionJournalError::Illegal {
            reason: "semantic delta before value disagrees",
            ..
        }
    ));
}

#[test]
fn missing_payload_after_a_bad_before_keeps_the_opened_head() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "chain-bad", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let empty_object = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            crate::journal_share::empty_datum(),
            "empty evidence",
        )
        .unwrap();
    let bad = journal
        .append(
            Some(&opened.head),
            ExecutionRecord::SemanticDelta {
                cause: "bad-before".into(),
                changes: vec![SemanticChange {
                    key: "answer".into(),
                    before: Some(content(9)),
                    after: Some(content(4)),
                }],
                evidence_set: crate::journal_share::empty_id(),
            },
            vec![empty_object],
        )
        .unwrap();
    let mut state = backend.read_state().unwrap();
    push_missing_payload(&mut state);
    let Err(fault) = crate::journal_replay::fold("chain-bad", &Limits::default(), &state) else {
        panic!("broken chain was accepted");
    };
    assert_eq!(fault.last.as_ref(), Some(&opened.head));
    assert_ne!(fault.last.as_ref(), Some(&bad));
    assert!(
        matches!(
            fault.error.as_ref(),
            ExecutionJournalError::Illegal {
                reason: "semantic delta before value disagrees",
                ..
            }
        ),
        "error={:?} head={:?}",
        fault.error,
        fault.last
    );
}

#[test]
fn missing_payload_after_a_valid_delta_keeps_that_head() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "chain-kept", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(content(4)),
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    let mut state = backend.read_state().unwrap();
    push_missing_payload(&mut state);
    let Err(fault) = crate::journal_replay::fold("chain-kept", &Limits::default(), &state) else {
        panic!("broken chain was accepted");
    };
    assert!(matches!(
        fault.error.as_ref(),
        ExecutionJournalError::Journal(JournalError::MissingPayload(_))
    ));
    assert_eq!(fault.last.as_ref(), Some(&delta));
}

#[test]
fn unmatched_receipt_keeps_a_valid_delta_as_the_verified_head() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "kept", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(content(4)),
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    let mut state = backend.read_state().unwrap();
    push_record(
        &mut state,
        "kept",
        ExecutionRecord::EffectReceipt {
            effect_id: "missing".into(),
            outcome: "done".into(),
            output: None,
        },
    );
    let Err(fault) = crate::journal_replay::fold("kept", &Limits::default(), &state) else {
        panic!("unmatched receipt was accepted");
    };
    assert_eq!(fault.last.as_ref(), Some(&delta));
    assert!(matches!(
        fault.error.as_ref(),
        ExecutionJournalError::Illegal {
            reason: "receipt must match one unresolved request",
            ..
        }
    ));
}

#[test]
fn repeated_snapshots_reuse_one_root_and_accept_a_deletion() {
    let journal = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "roots", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let fact = content(4);
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(fact.clone()),
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    journal.snapshot(&delta).unwrap();
    let head = journal.rebuild().unwrap().head;
    journal.snapshot(&head).unwrap();
    let head = journal.rebuild().unwrap().head;
    let DeltaAppend::Appended(removed) = journal
        .append_delta(
            &head,
            "fact/remove",
            vec![SemanticChange {
                key: "answer".into(),
                before: Some(fact),
                after: None,
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("deletion must append");
    };
    journal.snapshot(&removed).unwrap();
    let rebuilt = journal.rebuild().unwrap();
    assert!(rebuilt.causal.facts.is_empty());
    assert_eq!(rebuilt.causal_applies, 0);
    assert_eq!(rebuilt.snapshots.len(), 3);
    assert_eq!(rebuilt.snapshots[0].state, rebuilt.snapshots[1].state);
    assert_ne!(rebuilt.snapshots[1].state, rebuilt.snapshots[2].state);
}

#[test]
fn duplicate_evidence_identity_is_not_verified() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "repeat", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let member = content(9);
    let DeltaAppend::Appended(delta) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(content(4)),
            }],
            BTreeSet::from([member.clone()]),
        )
        .unwrap()
    else {
        panic!("delta must append");
    };
    let stored = backend.read_state().unwrap();
    let entry = stored.entries.get(&delta.sequence).unwrap();
    let (_, record) =
        crate::codec::decode_datum(stored.datums.get(&entry.payloads[0]).unwrap()).unwrap();
    let ExecutionRecord::SemanticDelta { evidence_set, .. } = record else {
        panic!("delta record");
    };
    let key = crate::journal_share::id_key(&member);
    let node = crate::journal_share::node_datum(&key, &member, 2, Some(&evidence_set), None);
    let prepared = journal
        .prepare_datum(crate::ObjectKind::EvidenceSet, node, "repeat")
        .unwrap();
    let repeated = journal
        .append(
            Some(&delta),
            ExecutionRecord::SemanticDelta {
                cause: "repeat".into(),
                changes: Vec::new(),
                evidence_set: prepared.reference.content.clone(),
            },
            vec![prepared],
        )
        .unwrap();
    let Err(fault) = journal.rebuild_report() else {
        panic!("repeated member was accepted");
    };
    assert_eq!(fault.last_verified_head.as_ref(), Some(&delta));
    assert_ne!(fault.last_verified_head.as_ref(), Some(&repeated));
    assert!(matches!(
        fault.error.as_ref(),
        ExecutionJournalError::Semantic("shared state")
    ));
}

fn rewrite_snapshot_evidence(state: &mut StoredState, execution: &str, evidence: &ContentId) {
    let head = state.head.clone().unwrap();
    let entry = state.entries.get(&head.sequence).unwrap().clone();
    let record_datum = state.datums.get(&entry.payloads[0]).unwrap().clone();
    let (_, record) = crate::codec::decode_datum(&record_datum).unwrap();
    let ExecutionRecord::Snapshot {
        covers,
        state: wrapper,
        ..
    } = record
    else {
        panic!("snapshot entry");
    };
    let (facts_root, _) =
        crate::journal_share::open_snapshot(state.datums.get(&wrapper).unwrap()).unwrap();
    let object =
        JournalObject::from_datum(crate::journal_share::snapshot_datum(&facts_root, evidence))
            .unwrap();
    let replacement = ExecutionRecord::Snapshot {
        covers,
        state: object.id.clone(),
        evidence_set: evidence.clone(),
    };
    let record_object =
        JournalObject::from_datum(crate::codec::encode_datum(execution, &replacement)).unwrap();
    let rewritten = JournalEntry::new(
        entry.sequence,
        entry.previous.clone(),
        entry.kind.clone(),
        vec![record_object.id.clone(), object.id.clone()],
    );
    state
        .datums
        .insert(record_object.id.clone(), record_object.datum().clone());
    state
        .objects
        .insert(record_object.id.clone(), record_object.bytes.clone());
    state
        .datums
        .insert(object.id.clone(), object.datum().clone());
    state
        .objects
        .insert(object.id.clone(), object.bytes.clone());
    state.head = Some(JournalHead {
        sequence: rewritten.sequence,
        entry: rewritten.id.clone(),
    });
    state.entries.insert(rewritten.sequence, rewritten);
}

fn push_missing_payload(state: &mut StoredState) {
    let head = state.head.clone().unwrap();
    let entry = JournalEntry::new(
        head.sequence + 1,
        Some(head.entry),
        Symbol::qualified("roadmap-execution", "semantic-delta"),
        vec![content(99)],
    );
    state.head = Some(JournalHead {
        sequence: entry.sequence,
        entry: entry.id.clone(),
    });
    state.entries.insert(entry.sequence, entry);
}

fn push_record(state: &mut StoredState, execution: &str, record: ExecutionRecord) {
    let head = state.head.clone().unwrap();
    let record_object =
        JournalObject::from_datum(crate::codec::encode_datum(execution, &record)).unwrap();
    let entry = JournalEntry::new(
        head.sequence + 1,
        Some(head.entry),
        Symbol::qualified("roadmap-execution", record.tag()),
        vec![record_object.id.clone()],
    );
    state
        .datums
        .insert(record_object.id.clone(), record_object.datum().clone());
    state
        .objects
        .insert(record_object.id.clone(), record_object.bytes.clone());
    state.head = Some(JournalHead {
        sequence: entry.sequence,
        entry: entry.id.clone(),
    });
    state.entries.insert(entry.sequence, entry);
}
