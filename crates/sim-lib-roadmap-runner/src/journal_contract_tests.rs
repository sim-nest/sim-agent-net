// conformance: the durable roadmap journal replays exactly and fences stale writers.

use std::sync::Arc;

use sim_kernel::{ContentId, Symbol};
use sim_lib_journal::{JournalError, MemoryBackend};

use crate::{ExecutionJournal, ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits};

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
