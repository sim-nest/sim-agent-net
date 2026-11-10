// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{collections::BTreeSet, sync::Arc, time::Instant};

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::{
    Journal, JournalBackend, JournalEntry, JournalHead, JournalObject, MemoryBackend,
};

use crate::journal_share::{self, empty_datum};
use crate::{
    DeltaAppend, ExecutionJournal, ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits,
    SemanticChange, codec,
};

fn content(bytes: [u8; 32]) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), bytes)
}

fn fact(index: u32) -> ContentId {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&index.to_le_bytes());
    content(bytes)
}

fn pins() -> ExecutionPins {
    ExecutionPins {
        conduct: "conduct".into(),
        policy: "policy".into(),
        source_deck: content([1; 32]),
        model_pick: "model".into(),
        runner_generation: "runner".into(),
    }
}

fn change(index: u32) -> SemanticChange {
    SemanticChange {
        key: format!("fact/{index:06}"),
        before: None,
        after: Some(fact(index)),
    }
}

fn admit_prefix(
    backend: &Arc<MemoryBackend>,
    execution: &str,
    opened: &JournalHead,
    count: u32,
) -> JournalHead {
    let empty = JournalObject::from_datum(empty_datum()).expect("empty evidence datum");
    assert_eq!(empty.id, journal_share::empty_id());
    // Each publish re-verifies the whole journal. One batch keeps this setup
    // linear. The timed check is the suffix rebuild after the snapshot.
    let mut previous = opened.clone();
    let mut objects = vec![empty.clone()];
    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count {
        let record = ExecutionRecord::SemanticDelta {
            cause: "fact/change".into(),
            changes: vec![change(index)],
            evidence_set: empty.id.clone(),
        };
        let object = JournalObject::from_datum(codec::encode_datum(execution, &record))
            .expect("canonical fact record");
        let entry = JournalEntry::new(
            previous.sequence + 1,
            Some(previous.entry.clone()),
            Symbol::qualified("roadmap-execution", record.tag()),
            vec![object.id.clone(), empty.id.clone()],
        );
        previous = JournalHead {
            sequence: entry.sequence,
            entry: entry.id.clone(),
        };
        objects.push(object);
        entries.push(entry);
    }
    let api = Journal::new(Arc::clone(backend));
    let lease = api.acquire_lease().expect("lease");
    let published = api
        .publish(&lease, Some(opened), objects, entries)
        .expect("prefix admission");
    assert_eq!(published, previous);
    previous
}

#[test]
fn three_deltas_after_a_snapshot_apply_only_the_suffix() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(Arc::clone(&backend), "suffix", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let evidence = content([9; 32]);
    let mut head = opened.head.clone();
    for index in 0..24 {
        let DeltaAppend::Appended(next) = journal
            .append_delta(
                &head,
                "fact/change",
                vec![change(index)],
                BTreeSet::from([evidence.clone()]),
            )
            .unwrap()
        else {
            panic!("fact delta must append");
        };
        head = next;
    }
    let first = journal.snapshot(&head).unwrap();
    let mut head = journal.rebuild().unwrap().head;
    for index in 24..27 {
        let DeltaAppend::Appended(next) = journal
            .append_delta(
                &head,
                "fact/change",
                vec![change(index)],
                BTreeSet::from([evidence.clone()]),
            )
            .unwrap()
        else {
            panic!("suffix delta must append");
        };
        head = next;
    }
    let cold = ExecutionJournal::new(Arc::clone(&backend), "suffix", Limits::default());
    let rebuilt = cold.rebuild().unwrap();
    assert_eq!(cold.rebuild_report().unwrap(), rebuilt);
    assert_eq!(rebuilt.causal_applies, 3);
    assert_eq!(rebuilt.causal.facts.len(), 27);
    assert_eq!(rebuilt.snapshots, vec![first.clone()]);
    assert_eq!(rebuilt.causal.facts.get("fact/000000"), Some(&fact(0)));
    assert_eq!(rebuilt.causal.facts.get("fact/000026"), Some(&fact(26)));
    let before = backend.read_state().unwrap().datums.len();
    let second = cold.snapshot(&rebuilt.head).unwrap();
    let added = backend.read_state().unwrap().datums.len() - before;
    assert!(
        added < 48,
        "second snapshot stored {added} objects for three key changes"
    );
    assert_ne!(second.state, first.state);
    let again = ExecutionJournal::new(backend, "suffix", Limits::default())
        .rebuild()
        .unwrap();
    assert_eq!(again.causal_applies, 0);
    assert_eq!(again.causal.facts.len(), 27);
}

#[test]
fn duplicate_fact_key_is_not_a_change() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(backend.clone(), "order", Limits::default());
    let value = fact(1);
    let key = "fact/000001";
    let leaf = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            journal_share::node_datum(key, &value, 1, None, None),
            "order",
        )
        .unwrap();
    let repeated = journal
        .prepare_datum(
            crate::ObjectKind::EvidenceSet,
            journal_share::node_datum(key, &value, 2, Some(&leaf.reference.content), None),
            "order",
        )
        .unwrap();
    let stored = backend.read_state().unwrap();
    let grown = journal_share::fact_changes(
        &journal_share::empty_id(),
        &leaf.reference.content,
        &stored.datums,
        &stored.objects,
    )
    .unwrap();
    assert_eq!(grown.get(key), Some(&Some(value)));
    let repeated = journal_share::fact_changes(
        &leaf.reference.content,
        &repeated.reference.content,
        &stored.datums,
        &stored.objects,
    );
    assert!(matches!(
        repeated,
        Err(ExecutionJournalError::Semantic("shared state"))
    ));
}

#[test]
fn cold_replay_of_one_hundred_thousand_records_applies_only_the_suffix() {
    const PREFIX: u32 = 100_000;
    let limits = Limits {
        max_execution_bytes: 512 * 1024 * 1024,
        ..Limits::default()
    };
    let backend = Arc::new(MemoryBackend::new());
    let journal = ExecutionJournal::new(Arc::clone(&backend), "scale", limits.clone());
    let opened = journal.open(pins(), None).unwrap();
    println!("admitting {PREFIX} prefix records in one batch");
    let admitted = Instant::now();
    let prefix_head = admit_prefix(&backend, "scale", &opened.head, PREFIX);
    println!("prefix admitted in {:?}", admitted.elapsed());
    let rebuilding = Instant::now();
    let sealed = journal.rebuild().unwrap();
    println!("prefix rebuild in {:?}", rebuilding.elapsed());
    assert_eq!(sealed.head, prefix_head);
    assert_eq!(sealed.causal.facts.len(), PREFIX as usize);
    assert_eq!(sealed.causal_applies, u64::from(PREFIX));
    println!("writing snapshot");
    let snapping = Instant::now();
    let first = journal.snapshot(&sealed.head).unwrap();
    println!("snapshot stored in {:?}", snapping.elapsed());
    let refreshing = Instant::now();
    let mut head = journal.rebuild().unwrap().head;
    println!("post-snapshot head in {:?}", refreshing.elapsed());
    for index in PREFIX..PREFIX + 3 {
        let DeltaAppend::Appended(next) = journal
            .append_delta(&head, "fact/change", vec![change(index)], BTreeSet::new())
            .unwrap()
        else {
            panic!("suffix delta must append");
        };
        head = next;
    }
    let before = backend.read_state().unwrap().datums.len();
    // Five cold suffix samples: 199.167358621s, 200.574472273s,
    // 202.704058608s, 201.863424817s, 200.9815069s. The ceiling is
    // ceil(202.704058608 * 1.15) milliseconds. A chain check that repeated
    // once per record takes an hour or more.
    const COLD_SUFFIX_CEILING_MS: u128 = 233_110;
    let mut samples = Vec::with_capacity(5);
    let mut rebuilt = None;
    for _ in 0..5 {
        let cold = ExecutionJournal::new(Arc::clone(&backend), "scale", limits.clone());
        let started = Instant::now();
        let once = cold.rebuild().unwrap();
        samples.push(started.elapsed());
        if let Some(previous) = &rebuilt {
            assert_eq!(once, *previous);
        }
        rebuilt = Some(once);
    }
    let rebuilt = rebuilt.expect("five cold samples");
    let cold = ExecutionJournal::new(Arc::clone(&backend), "scale", limits.clone());
    assert_eq!(cold.rebuild_report().unwrap(), rebuilt);
    assert_eq!(rebuilt.causal_applies, 3);
    assert_eq!(rebuilt.causal.facts.len(), PREFIX as usize + 3);
    assert_eq!(rebuilt.snapshots, vec![first]);
    println!("cold suffix samples: {samples:?}");
    let slowest = samples.iter().copied().max().expect("five cold samples");
    let _second = cold.snapshot(&rebuilt.head).unwrap();
    let added = backend.read_state().unwrap().datums.len() - before;
    assert!(
        added < 96,
        "snapshot of three new facts stored {added} objects"
    );
    assert!(
        slowest.as_millis() <= COLD_SUFFIX_CEILING_MS,
        "cold suffix replay of {PREFIX} records samples {samples:?}"
    );
}
