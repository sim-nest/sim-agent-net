//! Deterministic fake/cassette mission handoff runner.

use super::{
    mission::{AgentMission, AtelierAgentRole, key},
    radar::{RadarIndex, RadarQuery, RadarReport, retrieve_radar_hints},
};
use crate::{Decomposition, PlanningTask, Reflection, decompose_and_run, reflect};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use sim_lib_numbers_stats::bayesian_update_binary;
use sim_lib_stream_core::{DevCassette, DevEvent, LatencyClass};

/// Deterministic report from one fake/cassette mission handoff.
#[derive(Clone, Debug, PartialEq)]
pub struct MissionHandoffReport {
    /// Identifier of the mission this report covers.
    pub mission_id: Symbol,
    /// Ranked memory hints retrieved for the mission.
    pub radar: RadarReport,
    /// Planned subtasks and their produced outputs.
    pub decomposition: Decomposition,
    /// Reviewer reflection over the final output.
    pub reflection: Reflection,
    /// Bayesian-updated confidence in the handoff, in `[0, 1]`.
    pub confidence: f64,
    /// Recorded cassette of mission evidence events.
    pub evidence: DevCassette,
}

/// Runs a deterministic multi-agent handoff with ranked memory and cassette evidence.
pub fn run_mission_handoff(
    cx: &mut Cx,
    mission: &AgentMission,
    radar_index: &RadarIndex,
    runner: &dyn ModelRunner,
    max_steps: u32,
) -> Result<MissionHandoffReport> {
    let mut query = RadarQuery::new(mission.goal());
    query.repo = mission.allowed_repos().first().cloned();
    query.agent_role = Some(AtelierAgentRole::Cartographer.label().to_owned());
    let radar = retrieve_radar_hints(radar_index, &query)
        .map_err(|err| Error::Eval(format!("mission memory retrieval failed: {err}")))?;

    let goal = PlanningTask::new(mission.id().to_string(), mission.goal());
    let decomposition = decompose_and_run(cx, &goal, runner, max_steps)?;
    let output = decomposition
        .outputs
        .last()
        .ok_or_else(|| Error::Eval("mission produced no handoff output".to_owned()))?;
    let reflection = reflect(cx, output, runner, 0)?;
    let confidence = mission_confidence(&radar, &reflection)?;
    let evidence = DevCassette::from_events(
        mission.evidence_stream().clone(),
        evidence_events(mission, &radar, &decomposition, &reflection, confidence)?,
    )?;

    Ok(MissionHandoffReport {
        mission_id: mission.id().clone(),
        radar,
        decomposition,
        reflection,
        confidence,
        evidence,
    })
}

fn mission_confidence(radar: &RadarReport, reflection: &Reflection) -> Result<f64> {
    let prior = if radar.hints.is_empty() {
        0.5
    } else {
        radar.hints.iter().map(|hint| hint.confidence).sum::<f64>() / radar.hints.len() as f64
    };
    let (true_positive, false_positive) = if reflection.accept {
        (0.9, 0.2)
    } else {
        (0.45, 0.55)
    };
    bayesian_update_binary(prior, true_positive, false_positive)
        .map_err(|err| Error::Eval(format!("mission confidence failed: {err}")))
}

fn evidence_events(
    mission: &AgentMission,
    radar: &RadarReport,
    decomposition: &Decomposition,
    reflection: &Reflection,
    confidence: f64,
) -> Result<Vec<DevEvent>> {
    let mut events = vec![
        mission_event(
            "retrieve",
            AtelierAgentRole::Cartographer,
            Expr::List(
                radar
                    .hints
                    .iter()
                    .map(|hint| Expr::String(hint.chunk_id.clone()))
                    .collect(),
            ),
        )?,
        mission_event(
            "plan",
            AtelierAgentRole::Editor,
            Expr::List(
                decomposition
                    .subtasks
                    .iter()
                    .map(|task| Expr::String(task.id.clone()))
                    .collect(),
            ),
        )?,
        mission_event(
            "guard-result",
            AtelierAgentRole::Guard,
            Expr::List(
                mission
                    .capabilities()
                    .iter()
                    .map(|capability| Expr::String(format!("{capability:?}")))
                    .collect(),
            ),
        )?,
    ];

    let handoff_roles = [
        AtelierAgentRole::Editor,
        AtelierAgentRole::CodecSpecialist,
        AtelierAgentRole::Validator,
        AtelierAgentRole::DocsAgent,
        AtelierAgentRole::PinAgent,
        AtelierAgentRole::Reviewer,
    ];
    for (index, output) in decomposition.outputs.iter().enumerate() {
        events.push(mission_event(
            "handoff",
            handoff_roles[index % handoff_roles.len()].clone(),
            Expr::Map(vec![
                key("task", Expr::String(output.task.id.clone())),
                key("content", Expr::String(output.content.clone())),
            ]),
        )?);
    }
    for run in mission.validations() {
        events.push(mission_event(
            "validate",
            AtelierAgentRole::Validator,
            run.as_expr(),
        )?);
    }
    for run in mission.docs_runs() {
        events.push(mission_event(
            "docs-run",
            AtelierAgentRole::DocsAgent,
            run.as_expr(),
        )?);
    }
    for point in mission.decision_points() {
        events.push(mission_event(
            "human-gate",
            AtelierAgentRole::HumanGate,
            point.as_expr(),
        )?);
    }
    events.push(mission_event(
        "reflect",
        AtelierAgentRole::Reviewer,
        Expr::Map(vec![
            key("accept", Expr::Bool(reflection.accept)),
            key("critique", Expr::String(reflection.critique.clone())),
            key("confidence", Expr::String(format!("{confidence:.6}"))),
        ]),
    )?);
    Ok(events)
}

fn mission_event(kind: &str, role: AtelierAgentRole, payload: Expr) -> Result<DevEvent> {
    let latency = match role {
        AtelierAgentRole::Validator | AtelierAgentRole::DocsAgent => LatencyClass::OfflineRender,
        _ => LatencyClass::Interactive,
    };
    DevEvent::new(kind, role.as_symbol(), latency, payload)
}
