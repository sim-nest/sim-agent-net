//! Deterministic Mission Control fixtures for agent-operated Atelier runs.

use super::{
    AgentMission, GuardCapability, HumanDecisionPoint, MissionRun, WorkspaceLease,
    WorkspaceLeaseConflict, WorkspaceLeaseMode, detect_workspace_lease_conflicts,
};
use sim_kernel::{Expr, Result, Symbol};
use sim_lib_stream_core::{DevCassette, DevEvent, LatencyClass};

/// Fake Mission Control data used by UI and replay tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionControlFixture {
    /// Mission rendered by Mission Control.
    pub mission: AgentMission,
    /// Replayable evidence stream for the mission.
    pub cassette: DevCassette,
    /// Lease conflicts surfaced to the operator.
    pub conflicts: Vec<WorkspaceLeaseConflict>,
    /// Structured mission snapshot as SIM expression data.
    pub snapshot: Expr,
}

/// Builds a deterministic fake Mission Control fixture with no live model.
pub fn fake_mission_control_fixture() -> Result<MissionControlFixture> {
    let mission = mission();
    let conflicting = AgentMission::new(
        Symbol::qualified("agent/mission", "docs-refresh"),
        "sim-agent-net",
    )
    .with_goal("Refresh generated docs")
    .with_lease(WorkspaceLease::crate_name("sim-agent-net", "sim-lib-agent"));
    let conflicts = detect_workspace_lease_conflicts(&[mission.clone(), conflicting]);
    let cassette = evidence_cassette(mission.evidence_stream().clone())?;
    let snapshot = mission_snapshot(&mission, &conflicts);
    Ok(MissionControlFixture {
        mission,
        cassette,
        conflicts,
        snapshot,
    })
}

fn mission() -> AgentMission {
    AgentMission::new(
        Symbol::qualified("agent/mission", "mission-control-fixture"),
        "sim-agent-net",
    )
    .with_goal("Render Mission Control")
    .with_scope_summary("agent mission state and deterministic evidence")
    .with_capability(GuardCapability::EditRepo("sim-agent-net".to_owned()))
    .with_capability(GuardCapability::RunValidation("sim-agent-net".to_owned()))
    .with_capability(GuardCapability::RegenDocs("sim-agent-net".to_owned()))
    .with_lease(WorkspaceLease::crate_name("sim-agent-net", "sim-lib-agent"))
    .with_lease(
        WorkspaceLease::file(
            "sim-agent-net",
            "crates/sim-lib-agent/src/atelier/mission_control.rs",
        )
        .for_read(),
    )
    .with_lease(WorkspaceLease::ide_object(Symbol::qualified(
        "ide/object",
        "agent-mission-control",
    )))
    .with_validation(MissionRun::new(
        "agent-view-tests",
        "cargo test -p sim-lib-view-agent mission_control",
    ))
    .with_docs_run(MissionRun::new(
        "simdoc",
        "cargo run -p xtask -- simdoc --check",
    ))
    .with_decision_point(HumanDecisionPoint::new(
        "approve",
        "Approve the Mission Control change",
    ))
    .with_recipe_pattern(Symbol::new("a30-009-agentic-workflow"))
}

fn evidence_cassette(stream_id: Symbol) -> Result<DevCassette> {
    let node = Symbol::qualified("atelier/agent", "mission-control");
    let events = vec![
        DevEvent::new(
            "retrieval",
            node.clone(),
            LatencyClass::OfflineRender,
            payload("Cartographer ranked Mission Control context"),
        )?,
        DevEvent::new(
            "guard",
            node.clone(),
            LatencyClass::Interactive,
            payload("Guard accepted sim-agent-net lease"),
        )?,
        DevEvent::validate(
            node.clone(),
            payload("cargo test -p sim-lib-view-agent mission_control"),
        )?,
        DevEvent::new(
            "human-gate",
            node.clone(),
            LatencyClass::Interactive,
            payload("approval pending"),
        )?,
        DevEvent::new(
            "reflect",
            node,
            LatencyClass::OfflineRender,
            payload("F6 attribution cites cassette evidence"),
        )?,
    ];
    DevCassette::from_events(stream_id, events)
}

fn mission_snapshot(mission: &AgentMission, conflicts: &[WorkspaceLeaseConflict]) -> Expr {
    Expr::Map(vec![
        key("id", Expr::Symbol(mission.id().clone())),
        key("goal", Expr::String(mission.goal().to_owned())),
        key(
            "recipe-pattern",
            Expr::Symbol(mission.recipe_pattern().clone()),
        ),
        key(
            "roles",
            Expr::List(
                mission
                    .roles()
                    .iter()
                    .map(|role| Expr::Symbol(role.as_symbol()))
                    .collect(),
            ),
        ),
        key(
            "validations",
            Expr::List(
                mission
                    .validations()
                    .iter()
                    .map(|run| Expr::String(run.label.clone()))
                    .collect(),
            ),
        ),
        key(
            "human-gates",
            Expr::List(
                mission
                    .decision_points()
                    .iter()
                    .map(|point| Expr::String(point.id.clone()))
                    .collect(),
            ),
        ),
        key(
            "conflicts",
            Expr::List(conflicts.iter().map(conflict_expr).collect()),
        ),
    ])
}

fn conflict_expr(conflict: &WorkspaceLeaseConflict) -> Expr {
    Expr::Map(vec![
        key("left-mission", Expr::Symbol(conflict.left_mission.clone())),
        key(
            "right-mission",
            Expr::Symbol(conflict.right_mission.clone()),
        ),
        key("left-target", Expr::String(conflict.left.target.clone())),
        key("right-target", Expr::String(conflict.right.target.clone())),
        key(
            "mode",
            Expr::Symbol(Symbol::new(match conflict.left.mode {
                WorkspaceLeaseMode::SharedRead => "shared-read",
                WorkspaceLeaseMode::ExclusiveWrite => "exclusive-write",
            })),
        ),
    ])
}

fn payload(summary: &str) -> Expr {
    Expr::Map(vec![key("summary", Expr::String(summary.to_owned()))])
}

fn key(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
