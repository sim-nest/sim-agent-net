use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::{
    AgentMission, AtelierAgentRole, GuardCapability, HumanDecisionPoint, MissionRun, RadarChunk,
    RadarIndex, SourceSpan, WorkspaceLease, WorkspaceLeaseMode, detect_workspace_lease_conflicts,
    run_mission_handoff,
};
use crate::{ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, EncodeOptions, Expr, ReadPolicy, Result, Symbol,
};

struct ScriptedRunner {
    script: Mutex<VecDeque<Vec<Expr>>>,
    calls: AtomicUsize,
}

impl ScriptedRunner {
    fn new(script: impl IntoIterator<Item = Vec<Expr>>) -> Self {
        Self {
            script: Mutex::new(script.into_iter().collect()),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ModelRunner for ScriptedRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            Symbol::new("runner/mission-script"),
            "mission/script",
            Symbol::new("test"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, _cx: &mut sim_kernel::Cx, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let content = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted mission runner exhausted");
        Ok(ModelResponse::new(
            Symbol::new("runner/mission-script"),
            "mission/script",
            content,
            Symbol::new("stop"),
        ))
    }
}

#[test]
fn mission_descriptor_uses_sup56_slots_and_round_trips() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    let mission = mission_fixture();
    let descriptor = mission.descriptor();

    assert_eq!(descriptor.id, Symbol::qualified("agent/mission", "sup2-10"));
    assert_eq!(descriptor.memory[0].name, Symbol::new("f2-radar-retrieve"));
    assert_eq!(descriptor.plan[0].name, Symbol::new("f3-decompose"));
    assert_eq!(descriptor.evaluation[0].name, Symbol::new("f3-reflect"));
    assert_eq!(descriptor.trace[0].name, Symbol::new("dev-cassette-ledger"));
    assert!(
        descriptor
            .act
            .iter()
            .any(|slot| slot.name == AtelierAgentRole::HumanGate.as_symbol())
    );

    let expr = descriptor.as_expr();
    for codec in [
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "lisp"),
    ] {
        let encoded = encode_with_codec(&mut cx, &codec, &expr, EncodeOptions::default()).unwrap();
        let decoded = decode_with_codec(
            &mut cx,
            &codec,
            match encoded {
                sim_codec::Output::Text(text) => Input::Text(text),
                sim_codec::Output::Bytes(bytes) => Input::Bytes(bytes),
            },
            ReadPolicy::default(),
        )
        .unwrap();
        assert!(decoded.canonical_eq(&expr));
    }
}

#[test]
fn workspace_lease_conflicts_detect_overlapping_write_claims() {
    let left = AgentMission::new(Symbol::qualified("agent/mission", "left"), "sim-agent-net")
        .with_lease(WorkspaceLease::file(
            "sim-agent-net",
            "crates/sim-lib-agent/src/atelier/mission.rs",
        ));
    let right =
        AgentMission::new(Symbol::qualified("agent/mission", "right"), "sim-agent-net").with_lease(
            WorkspaceLease::directory("sim-agent-net", "crates/sim-lib-agent/src/atelier"),
        );
    let reader = AgentMission::new(
        Symbol::qualified("agent/mission", "reader"),
        "sim-agent-net",
    )
    .with_lease(
        WorkspaceLease::file(
            "sim-agent-net",
            "crates/sim-lib-agent/src/atelier/mission.rs",
        )
        .for_read(),
    );

    let conflicts = detect_workspace_lease_conflicts(&[left.clone(), right.clone()]);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].left_mission, *left.id());
    assert_eq!(conflicts[0].right_mission, *right.id());

    let shared_reads = detect_workspace_lease_conflicts(&[
        reader.clone(),
        reader
            .clone()
            .with_evidence_stream(Symbol::qualified("atelier/dev", "reader-two")),
    ]);
    assert!(shared_reads.is_empty());
    assert_eq!(reader.leases()[0].mode, WorkspaceLeaseMode::SharedRead);
}

#[test]
fn fake_runner_handoff_records_dev_cassette_without_network() {
    let runner = ScriptedRunner::new([
        vec![Expr::List(vec![
            task_expr("retrieve", "Retrieve ranked context"),
            task_expr("edit", "Apply scoped edit"),
        ])],
        vec![Expr::String("retrieved context".to_owned())],
        vec![Expr::String("edited mission code".to_owned())],
        vec![Expr::Map(vec![
            (Expr::Symbol(Symbol::new("accept")), Expr::Bool(true)),
            (
                Expr::Symbol(Symbol::new("critique")),
                Expr::String("evidence matches leases".to_owned()),
            ),
        ])],
    ]);
    let mut cx = eval_cx();

    let report =
        run_mission_handoff(&mut cx, &mission_fixture(), &radar_fixture(), &runner, 3).unwrap();

    assert_eq!(runner.calls(), 4);
    assert_eq!(report.decomposition.subtasks.len(), 2);
    assert!(report.reflection.accept);
    assert!(report.confidence > 0.5);
    assert_eq!(
        report.evidence.cassette().envelopes().len(),
        9,
        "retrieve, plan, guard, two handoffs, validation, docs, human gate, reflect"
    );
    assert_eq!(
        report.evidence.content_hash(),
        report.evidence.replay_content_hash().unwrap()
    );
}

fn mission_fixture() -> AgentMission {
    AgentMission::new(
        Symbol::qualified("agent/mission", "sup2-10"),
        "sim-agent-net",
    )
    .with_goal("Implement the Atelier mission model")
    .with_scope_summary("mission data and deterministic handoff in sim-lib-agent")
    .with_capability(GuardCapability::EditRepo("sim-agent-net".to_owned()))
    .with_capability(GuardCapability::RunValidation("sim-agent-net".to_owned()))
    .with_lease(WorkspaceLease::crate_name("sim-agent-net", "sim-lib-agent"))
    .with_validation(MissionRun::new(
        "agent-tests",
        "cargo test -p sim-lib-agent mission",
    ))
    .with_docs_run(MissionRun::new(
        "simdoc",
        "cargo run -p xtask -- simdoc --check",
    ))
    .with_decision_point(HumanDecisionPoint::new(
        "push",
        "Confirm commits and pins before push",
    ))
    .with_recipe_pattern(Symbol::new("a30-009-agentic-workflow"))
}

fn radar_fixture() -> RadarIndex {
    let mut chunk = RadarChunk::new(
        "mission-contract",
        "Mission contract",
        SourceSpan {
            repo: "sim-agent-net".to_owned(),
            path: "crates/sim-lib-agent/src/atelier/mission.rs".to_owned(),
            line: 1,
        },
        "rustdoc",
        "Cartographer retrieves mission leases and agent pattern descriptors",
    );
    chunk.agent_roles = vec!["cartographer".to_owned()];
    RadarIndex {
        chunks: vec![chunk],
    }
}

fn task_expr(id: &str, prompt: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("id")), Expr::String(id.to_owned())),
        (
            Expr::Symbol(Symbol::new("prompt")),
            Expr::String(prompt.to_owned()),
        ),
    ])
}

fn eval_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

fn install_roundtrip_codecs(cx: &mut Cx) {
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
}
