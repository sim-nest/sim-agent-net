use sim_kernel::{Expr, Symbol};
use sim_lib_stream_core::StreamPacket;

use super::{
    AgentMission, AtelierToolAction, DocsOperation, DocsRegenerationRequest, GuardCapability,
    GuardDecision, PinUpdateRequest, ToolRunEvidence, evaluate_atelier_tool, repo_docs_descriptor,
    repo_validation_descriptor, simctl_tool_descriptors,
};

#[test]
fn simctl_tool_descriptors_cover_required_commands() {
    let descriptors = simctl_tool_descriptors("repo-control", "repo-docs");
    let ids = descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();

    for expected in [
        "simctl/clone",
        "simctl/meta-build",
        "simctl/audit",
        "simctl/no-github-check",
        "simctl/site",
        "simctl/repos",
        "simctl/atelier-site",
        "simctl/atelier-index",
        "simctl/atelier-radar",
        "simctl/atelier-guard",
    ] {
        assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
    }
}

#[test]
fn validation_tool_records_validate_envelope() {
    let descriptor = repo_validation_descriptor(
        "sim-agent-net",
        "cargo fmt --check && cargo test --workspace",
    );
    assert_eq!(
        descriptor.command,
        "cargo fmt --check && cargo test --workspace"
    );
    let mission = AgentMission::new(Symbol::qualified("agent/mission", "tools"), "sim-agent-net")
        .with_capability(GuardCapability::RunValidation("sim-agent-net".to_owned()));

    let eval = evaluate_atelier_tool(
        &mission,
        AtelierToolAction::Validation(ToolRunEvidence::new(
            descriptor,
            0,
            ".sim/atelier/logs/validation-sim-agent-net.log",
        )),
        Symbol::qualified("atelier/agent", "validator"),
        Symbol::qualified("atelier/dev", "tools"),
    )
    .unwrap();

    assert!(eval.decision.is_granted());
    assert_eq!(
        cassette_event_kind(&eval.cassette),
        Symbol::qualified("ide/event", "validate")
    );
    let payload = cassette_payload(&eval.cassette);
    assert_eq!(
        map_string(payload, "command"),
        "cargo fmt --check && cargo test --workspace"
    );
    assert_eq!(map_string(payload, "exit-status"), "0");
    assert_eq!(
        map_string(payload, "log-path"),
        ".sim/atelier/logs/validation-sim-agent-net.log"
    );
}

#[test]
fn generated_doc_hand_edit_is_refused_and_recorded() {
    let mission = AgentMission::new(Symbol::qualified("agent/mission", "docs"), "sim-agent-net")
        .with_capability(GuardCapability::RegenDocs("sim-agent-net".to_owned()));
    let eval = evaluate_atelier_tool(
        &mission,
        AtelierToolAction::DocsRegeneration(DocsRegenerationRequest {
            repo: "sim-agent-net".to_owned(),
            path: "docs/generated/repo-contract.json".to_owned(),
            docs_command: "cargo run -p xtask -- simdoc --check".to_owned(),
            generated_public_doc: true,
            operation: DocsOperation::HandEdit,
        }),
        Symbol::qualified("atelier/agent", "docs"),
        Symbol::qualified("atelier/dev", "docs"),
    )
    .unwrap();

    let GuardDecision::Refused(refusal) = &eval.decision else {
        panic!("expected generated docs refusal");
    };
    assert!(refusal.reason().contains("generated public docs"));
    assert_eq!(
        cassette_event_kind(&eval.cassette),
        Symbol::qualified("ide/event", "refusal")
    );
}

#[test]
fn docs_tool_records_docs_envelope() {
    let descriptor = repo_docs_descriptor("sim-tooling", "cargo run -p xtask -- simdoc --check");
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "docs-run"),
        "sim-tooling",
    )
    .with_capability(GuardCapability::RegenDocs("sim-tooling".to_owned()));

    let eval = evaluate_atelier_tool(
        &mission,
        AtelierToolAction::Docs(ToolRunEvidence::new(
            descriptor,
            0,
            ".sim/atelier/logs/docs-sim-tooling.log",
        )),
        Symbol::qualified("atelier/agent", "docs"),
        Symbol::qualified("atelier/dev", "docs-run"),
    )
    .unwrap();

    assert!(eval.decision.is_granted());
    assert_eq!(
        cassette_event_kind(&eval.cassette),
        Symbol::qualified("ide/event", "docs")
    );
    let payload = cassette_payload(&eval.cassette);
    assert_eq!(
        map_string(payload, "command"),
        "cargo run -p xtask -- simdoc --check"
    );
    assert_eq!(map_string(payload, "exit-status"), "0");
    assert_eq!(
        map_string(payload, "log-path"),
        ".sim/atelier/logs/docs-sim-tooling.log"
    );
}

#[test]
fn pin_update_requires_planpin_and_pushed_commit() {
    let request = PinUpdateRequest {
        repo: "sim-agent-net".to_owned(),
        current_commit: "aaaa".to_owned(),
        new_commit: "bbbb".to_owned(),
        pushed_commit_exists: true,
    };
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "pin-missing"),
        "sim-agent-net",
    );
    let missing = evaluate_atelier_tool(
        &mission,
        AtelierToolAction::PinUpdate(request.clone()),
        Symbol::qualified("atelier/agent", "pin"),
        Symbol::qualified("atelier/dev", "pin-missing"),
    )
    .unwrap();
    assert!(matches!(missing.decision, GuardDecision::Refused(_)));

    let mission = mission.with_capability(GuardCapability::PlanPin);
    let not_pushed = evaluate_atelier_tool(
        &mission,
        AtelierToolAction::PinUpdate(PinUpdateRequest {
            pushed_commit_exists: false,
            ..request
        }),
        Symbol::qualified("atelier/agent", "pin"),
        Symbol::qualified("atelier/dev", "pin-not-pushed"),
    )
    .unwrap();
    let GuardDecision::Refused(refusal) = &not_pushed.decision else {
        panic!("expected pushed commit refusal");
    };
    assert!(refusal.reason().contains("pushed upstream commit"));
}

#[test]
fn docs_descriptor_uses_exact_docs_command() {
    let descriptor = repo_docs_descriptor("sim-tooling", "cargo run -p xtask -- simdoc --check");
    assert_eq!(descriptor.id, "docs/sim-tooling");
    assert_eq!(descriptor.command, "cargo run -p xtask -- simdoc --check");
    assert_eq!(
        descriptor.required_capability,
        GuardCapability::RegenDocs("sim-tooling".to_owned())
    );
}

fn cassette_event_kind(cassette: &sim_lib_stream_core::DevCassette) -> Symbol {
    let envelope = &cassette.cassette().envelopes()[0];
    let StreamPacket::Data(packet) = envelope.packet() else {
        panic!("expected data packet");
    };
    packet.kind.clone()
}

fn cassette_payload(cassette: &sim_lib_stream_core::DevCassette) -> &Expr {
    let envelope = &cassette.cassette().envelopes()[0];
    let StreamPacket::Data(packet) = envelope.packet() else {
        panic!("expected data packet");
    };
    map_value(&packet.payload, "payload")
}

fn map_string<'a>(expr: &'a Expr, key: &str) -> &'a str {
    let Expr::String(value) = map_value(expr, key) else {
        panic!("expected string field {key}");
    };
    value
}

fn map_value<'a>(expr: &'a Expr, key: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map");
    };
    entries
        .iter()
        .find_map(|(entry_key, value)| {
            let Expr::Symbol(symbol) = entry_key else {
                return None;
            };
            (symbol.namespace.is_none() && symbol.name.as_ref() == key).then_some(value)
        })
        .unwrap_or_else(|| panic!("missing field {key}"))
}
