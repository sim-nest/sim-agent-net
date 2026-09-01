use sim_citizen::{CitizenRegistry, run_registry_conformance_expecting};
use sim_kernel::{Expr, Symbol, testing::bare_cx as cx};

use crate::*;

#[test]
fn every_public_record_has_shape_and_general_codec_round_trip() {
    let mut registry = CitizenRegistry::new();
    register_citizens(&mut registry).unwrap();
    let expected = [
        "agent/RunState",
        "agent/Outcome",
        "agent/Stop",
        "agent/UsageBudget",
        "agent/Usage",
        "agent/Event",
        "agent/JournalHead",
        "agent/JournalRecord",
        "agent/StepCard",
        "agent/ConductContract",
        "agent/RunFrame",
    ];
    run_registry_conformance_expecting(&mut cx(), &registry, &expected).unwrap();
}

#[test]
fn hostile_open_symbols_round_trip_unchanged() {
    let value = AgentEvent::new(
        Symbol::qualified("third-party.event", "x/../../../unknown"),
        Expr::Map(vec![(
            Expr::Symbol(Symbol::qualified("hostile", "key")),
            Expr::Bytes(vec![0, 255]),
        )]),
    );
    assert_eq!(
        value.kind.to_string(),
        "third-party.event/x/../../../unknown"
    );
    assert!(symbols::event::is_kind(&symbols::event::STEP_COMPLETED()));
    assert!(!symbols::event::is_kind(&value.kind));
}

#[test]
fn run_state_requires_standard_entries_and_qualified_extensions() {
    let mut entries = AgentRunState::standard().entries().to_vec();
    entries.push((Symbol::qualified("vendor", "trace"), Expr::Bool(true)));
    assert!(AgentRunState::new(entries.clone()).is_ok());
    entries.push((Symbol::qualified("vendor", "trace"), Expr::Bool(false)));
    assert!(
        AgentRunState::new(entries)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    let mut unqualified = AgentRunState::standard().entries().to_vec();
    unqualified.push((Symbol::new("secret"), Expr::Nil));
    assert!(
        AgentRunState::new(unqualified)
            .unwrap_err()
            .to_string()
            .contains("namespaced")
    );
}

#[test]
fn run_state_upsert_replaces_only_namespaced_observations() {
    let mut state = AgentRunState::standard();
    let key = Symbol::qualified("agent.provider", "seat");
    state
        .upsert(key.clone(), Expr::String("seat:a#one".into()))
        .unwrap();
    state
        .upsert(key.clone(), Expr::String("seat:a#two".into()))
        .unwrap();
    assert_eq!(state.get(&key), Some(&Expr::String("seat:a#two".into())));
    assert!(state.upsert(Symbol::new("secret"), Expr::Nil).is_err());
}

#[test]
fn usage_is_integer_unit_qualified_and_budget_layers_narrow_pointwise() {
    let turns = symbols::usage::MODEL_TURN();
    let tools = symbols::usage::TOOL_CALL();
    let caller = AgentUsageBudget::new(vec![UsageQuantity {
        unit: turns.clone(),
        amount: 5,
    }])
    .unwrap();
    let agent = AgentUsageBudget::new(vec![
        UsageQuantity {
            unit: turns.clone(),
            amount: 3,
        },
        UsageQuantity {
            unit: tools.clone(),
            amount: 2,
        },
    ])
    .unwrap();
    let effective = caller.narrow(&agent);
    assert_eq!(
        effective
            .limits()
            .iter()
            .find(|q| q.unit == turns)
            .unwrap()
            .amount,
        3
    );
    let mut usage = AgentUsage::default();
    usage
        .charge(
            &effective,
            UsageQuantity {
                unit: tools.clone(),
                amount: 2,
            },
        )
        .unwrap();
    assert!(
        usage
            .charge(
                &effective,
                UsageQuantity {
                    unit: tools,
                    amount: 1
                }
            )
            .is_err()
    );
    assert_eq!(
        symbols::currency_micro_units("sek").to_string(),
        "agent.usage/currency/SEK/micro-unit"
    );
}

#[test]
fn journal_rejects_gaps_divergence_malformed_hashes_and_handles_large_sequences() {
    let frame = AgentRunFrame::default();
    let mut journal = AgentJournal::new("graph", "bindings");
    journal
        .append(
            AgentEvent::default(),
            frame,
            AgentUsage::default(),
            vec![Expr::String("receipt".into())],
            Expr::Symbol(Symbol::qualified("topology", "continue")),
        )
        .unwrap();
    journal.verify().unwrap();
    let original = journal.records()[0].clone();
    journal.insert(original.clone()).unwrap();
    let mut divergent = original.clone();
    divergent.continuation = Expr::Nil;
    assert_eq!(
        journal.insert(divergent),
        Err(JournalError::DivergentDuplicate { sequence: 0 })
    );
    let mut malformed = original;
    malformed.sequence = u64::MAX;
    malformed.hash = "not-a-hash".into();
    assert!(matches!(
        AgentJournal::new("graph", "bindings").insert(malformed),
        Err(JournalError::Sequence {
            actual: u64::MAX,
            ..
        })
    ));
}

#[test]
fn sha256_matches_standard_vector() {
    assert_eq!(
        crate::sha256::digest_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
