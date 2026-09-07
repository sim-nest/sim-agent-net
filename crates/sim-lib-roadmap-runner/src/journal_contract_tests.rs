// conformance: the durable roadmap journal replays exactly and fences stale writers.

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::{
    Admission, JournalBackend, JournalError, JournalHead, JournalObject, Lease, MemoryBackend,
    StoredDatumRef, StoredState,
};

use crate::{
    DeltaAppend, ExecutionJournal, ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits,
    SemanticChange, TelemetryEvent, TelemetrySink,
};

fn content(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [byte; 32])
}

fn pins() -> ExecutionPins {
    ExecutionPins {
        conduct: "conduct".into(),
        policy: "policy".into(),
        source_deck: content(1),
        model_pick: "model".into(),
        runner_generation: "runner".into(),
    }
}

#[test]
fn replay_is_exact_and_a_stale_head_cannot_append() {
    let journal = ExecutionJournal::new(
        Arc::new(MemoryBackend::new()),
        "execution",
        Limits::default(),
    );
    let opened = journal.open(pins(), None).unwrap();
    let head = journal
        .append(
            Some(&opened.head),
            ExecutionRecord::StateTransition {
                from: "planned".into(),
                to: "running".into(),
            },
            vec![],
        )
        .unwrap();

    assert_eq!(journal.rebuild().unwrap(), journal.rebuild().unwrap());
    assert!(matches!(
        journal.append(
            Some(&opened.head),
            ExecutionRecord::Ambiguity {
                reason: "stale writer".into(),
            },
            vec![],
        ),
        Err(ExecutionJournalError::Journal(
            JournalError::WrongHead | JournalError::ConflictingDelivery
        ))
    ));
    assert_eq!(journal.rebuild().unwrap().head, head);
}

#[test]
fn cause_only_deltas_store_one_evidence_root_and_ten_thousand_noops_emit_nothing() {
    let journal =
        ExecutionJournal::new(Arc::new(MemoryBackend::new()), "causal", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let fact = content(8);
    let evidence = content(9);
    let changed = journal
        .append_delta(
            &opened.head,
            "observation/accepted",
            vec![SemanticChange {
                key: "phase/leaf".into(),
                before: None,
                after: Some(fact.clone()),
            }],
            BTreeSet::from([evidence.clone()]),
        )
        .unwrap();
    let DeltaAppend::Appended(head) = changed else {
        panic!("first semantic change must append")
    };
    for _ in 0..10_000 {
        assert_eq!(
            journal
                .append_delta(
                    &head,
                    "unchanged/revision",
                    vec![SemanticChange {
                        key: "phase/leaf".into(),
                        before: Some(fact.clone()),
                        after: Some(fact.clone()),
                    }],
                    BTreeSet::from([evidence.clone()]),
                )
                .unwrap(),
            DeltaAppend::Unchanged(head.clone())
        );
    }
    let rebuilt = journal.rebuild().unwrap();
    assert_eq!(rebuilt.records.len(), 2);
    assert_eq!(rebuilt.causal.facts.get("phase/leaf"), Some(&fact));
    assert_eq!(rebuilt.causal.evidence, BTreeSet::from([evidence]));
    assert_eq!(rebuilt.retention.entries.len(), 2);
    assert_eq!(rebuilt.retention.objects.len(), 3);
}

#[test]
fn verified_snapshot_rebuilds_and_missing_snapshot_content_fails_closed() {
    let backend = Arc::new(TamperBackend::default());
    let journal = ExecutionJournal::new(backend.clone(), "snapshot", Limits::default());
    let opened = journal.open(pins(), None).unwrap();
    let fact = content(4);
    let DeltaAppend::Appended(delta_head) = journal
        .append_delta(
            &opened.head,
            "fact/change",
            vec![SemanticChange {
                key: "answer".into(),
                before: None,
                after: Some(fact),
            }],
            BTreeSet::new(),
        )
        .unwrap()
    else {
        panic!("delta must append")
    };
    let snapshot = journal.snapshot(&delta_head).unwrap();
    let rebuilt = journal.rebuild().unwrap();
    assert_eq!(rebuilt.snapshots, vec![snapshot]);

    backend.tamper.store(true, Ordering::SeqCst);
    assert!(matches!(
        journal.rebuild(),
        Err(ExecutionJournalError::MissingObject)
            | Err(ExecutionJournalError::Journal(
                JournalError::MissingSemanticObject(_) | JournalError::MissingPayload(_)
            ))
    ));
}

#[test]
fn telemetry_sink_is_separate_from_durable_record_count() {
    #[derive(Default)]
    struct Sink(Vec<TelemetryEvent>);
    impl TelemetrySink for Sink {
        fn emit(&mut self, event: TelemetryEvent) {
            self.0.push(event);
        }
    }
    let journal = ExecutionJournal::new(
        Arc::new(MemoryBackend::new()),
        "telemetry",
        Limits::default(),
    );
    journal.open(pins(), None).unwrap();
    let before = journal.rebuild().unwrap().records.len();
    let mut sink = Sink::default();
    sink.emit(TelemetryEvent {
        name: "replay-ns".into(),
        value: 42,
    });
    assert_eq!(sink.0.len(), 1);
    assert_eq!(journal.rebuild().unwrap().records.len(), before);
}

#[derive(Default)]
struct TamperBackend {
    inner: MemoryBackend,
    tamper: AtomicBool,
}

impl JournalBackend for TamperBackend {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        self.inner.acquire_lease()
    }
    fn read_state(&self) -> Result<StoredState, JournalError> {
        let mut state = self.inner.read_state()?;
        if self.tamper.load(Ordering::SeqCst) {
            let snapshot = state.datums.iter().find_map(|(id, value)| {
                matches!(value, Datum::Node { tag, .. } if *tag == Symbol::qualified("roadmap-execution", "snapshot-v1"))
                    .then_some(id.clone())
            });
            if let Some(snapshot) = snapshot {
                state.datums.remove(&snapshot);
                state.objects.remove(&snapshot);
            }
        }
        Ok(state)
    }
    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
        self.inner.admit(admission)
    }
    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
        self.inner.put_datum(object)
    }
    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
        self.inner.get_datum(meaning)
    }
    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
        self.inner.rebuild_datum_index()
    }
}
