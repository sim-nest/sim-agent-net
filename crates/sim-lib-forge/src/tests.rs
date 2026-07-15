use sim_citizen::CitizenField;
use sim_kernel::{ContentId, Expr, Symbol};

use crate::{CompiledIntent, IntentStatus};

fn content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

#[test]
fn default_compiled_intent_starts_as_candidate() {
    let intent = CompiledIntent::default();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert!(intent.compiler_card.is_none());
    assert!(intent.approval.is_none());
}

#[test]
fn verified_and_golden_statuses_are_distinct() {
    assert_ne!(IntentStatus::Verified, IntentStatus::Golden);
    assert_eq!(
        IntentStatus::from_symbol(&Symbol::qualified("forge", "verified")),
        Some(IntentStatus::Verified)
    );
    assert_eq!(
        IntentStatus::from_symbol(&Symbol::qualified("forge", "golden")),
        Some(IntentStatus::Golden)
    );
}

#[test]
fn status_round_trips_as_symbol_field() {
    let encoded = IntentStatus::Golden.encode_field();
    assert_eq!(encoded, Expr::Symbol(Symbol::qualified("forge", "golden")));

    let decoded = IntentStatus::decode_field_expr(&encoded, "status").unwrap();
    assert_eq!(decoded, IntentStatus::Golden);
}

#[test]
fn compiled_intent_keeps_content_ids_and_human_approval_separate() {
    let intent = CompiledIntent {
        name: Symbol::qualified("forge", "summarize"),
        version: 3,
        source: content_id(10),
        packet: content_id(11),
        verifiers: vec![Symbol::qualified("bridge", "vote")],
        probes: vec![content_id(12)],
        status: IntentStatus::Verified,
        compiler_card: Some(content_id(13)),
        approval: None,
    };

    assert_eq!(intent.source, content_id(10));
    assert_eq!(intent.packet, content_id(11));
    assert_eq!(intent.status, IntentStatus::Verified);
    assert_ne!(intent.status, IntentStatus::Golden);
    assert!(intent.approval.is_none());
}
