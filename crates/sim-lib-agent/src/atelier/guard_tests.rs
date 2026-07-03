use sim_kernel::Symbol;
use sim_lib_stream_core::StreamPacket;

use super::{AgentMission, AtelierAction, GuardCapability, GuardDecision, evaluate_guarded_action};

fn mission_with(capability: GuardCapability) -> AgentMission {
    AgentMission::new(
        Symbol::qualified("agent/mission", "guard-test"),
        "sim-agent-net",
    )
    .with_capability(capability)
}

fn assert_refusal(eval: &super::GuardEvaluation) -> &str {
    let GuardDecision::Refused(refusal) = eval.decision() else {
        panic!("expected guard refusal");
    };
    let cassette = eval.cassette().expect("refusal should record a cassette");
    let envelope = &cassette.cassette().envelopes()[0];
    let StreamPacket::Data(packet) = envelope.packet() else {
        panic!("refusal should record a data packet");
    };
    assert_eq!(packet.kind, Symbol::qualified("ide/event", "refusal"));
    refusal.reason()
}

#[test]
fn leased_repo_edit_with_token_is_granted() {
    let mission = mission_with(GuardCapability::EditRepo("sim-agent-net".to_owned()));
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::edit_file("sim-agent-net", "crates/sim-lib-agent/src/atelier.rs"),
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "granted"),
    )
    .unwrap();

    assert!(eval.decision().is_granted());
    assert!(eval.cassette().is_none());
}

#[test]
fn edit_outside_lease_is_refused() {
    let mission = mission_with(GuardCapability::EditRepo("sim-stream".to_owned()));
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::edit_file("sim-stream", "crates/sim-lib-stream-core/src/dev.rs"),
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "outside-lease"),
    )
    .unwrap();

    assert!(assert_refusal(&eval).contains("mission lease"));
}

#[test]
fn meta_workspace_edit_is_refused_even_with_repo_token() {
    let mission = mission_with(GuardCapability::EditRepo("sim-agent-net".to_owned()));
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::edit_file(
            "sim-agent-net",
            ".meta-workspace/packages/sim-lib-agent/src/lib.rs",
        ),
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "meta-workspace"),
    )
    .unwrap();

    assert!(assert_refusal(&eval).contains(".meta-workspace"));
}

#[test]
fn github_remote_action_is_refused_unconditionally() {
    let mission = mission_with(GuardCapability::PlanPin);
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::AddGithubRemote {
            remote: "git@example.com:sim/sim-agent-net".to_owned(),
        },
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "github"),
    )
    .unwrap();

    assert!(assert_refusal(&eval).contains("GitHub remote"));
}

#[test]
fn publish_flag_and_mirror_actions_are_refused_unconditionally() {
    let mission = mission_with(GuardCapability::PlanPin);
    for (action, expected) in [
        (
            AtelierAction::FlipPublishToGithub {
                repo: "sim-agent-net".to_owned(),
            },
            "publish_to_github",
        ),
        (AtelierAction::PushMirrorRemote, "mirror remote"),
    ] {
        let eval = evaluate_guarded_action(
            &mission,
            action,
            Symbol::qualified("atelier/node", "guard"),
            Symbol::qualified("atelier/dev", "red-line"),
        )
        .unwrap();

        assert!(assert_refusal(&eval).contains(expected));
    }
}

#[test]
fn code_free_repos_reject_rust_edits() {
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "guard-test"),
        "repo-control",
    )
    .with_code_free_repo("repo-control")
    .with_capability(GuardCapability::EditRepo("repo-control".to_owned()));
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::edit_file("repo-control", "src/lib.rs"),
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "code-free"),
    )
    .unwrap();

    assert!(assert_refusal(&eval).contains("Rust source"));
}

#[test]
fn missing_token_records_refusal() {
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "guard-test"),
        "sim-agent-net",
    );
    let eval = evaluate_guarded_action(
        &mission,
        AtelierAction::RunValidation {
            repo: "sim-agent-net".to_owned(),
        },
        Symbol::qualified("atelier/node", "guard"),
        Symbol::qualified("atelier/dev", "missing-token"),
    )
    .unwrap();

    assert!(assert_refusal(&eval).contains("missing guard capability"));
}
