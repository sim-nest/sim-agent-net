use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::support::eval_cx;
use crate::{
    ModelCard, ModelRequest, ModelResponse, ModelRunner, PlanningOutput, PlanningTask, decompose,
    decompose_and_run, reflect,
};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};

struct ScriptedRunner {
    script: Mutex<VecDeque<ScriptStep>>,
    calls: AtomicUsize,
}

enum ScriptStep {
    Response(Vec<Expr>),
    Error(&'static str),
}

impl ScriptedRunner {
    fn new(script: impl IntoIterator<Item = ScriptStep>) -> Self {
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
            Symbol::new("runner/scripted"),
            "scripted/model",
            Symbol::new("test"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let step = self
            .script
            .lock()
            .map_err(|_| Error::PoisonedLock("scripted planning runner"))?
            .pop_front()
            .ok_or_else(|| Error::Eval("scripted planning runner exhausted".to_owned()))?;
        match step {
            ScriptStep::Response(content) => Ok(ModelResponse::new(
                Symbol::new("runner/scripted"),
                "scripted/model",
                content,
                Symbol::new("stop"),
            )),
            ScriptStep::Error(message) => Err(Error::Eval(message.to_owned())),
        }
    }
}

#[test]
fn decompose_and_run_executes_ordered_subtasks() {
    let runner = ScriptedRunner::new([
        response(vec![Expr::List(vec![
            task_expr("collect", "Collect evidence"),
            task_expr("summarize", "Summarize evidence"),
        ])]),
        response(vec![Expr::String("collected".to_owned())]),
        response(vec![Expr::String("summarized".to_owned())]),
    ]);
    let mut cx = eval_cx();
    let goal = PlanningTask::new("goal", "Prepare answer");

    let report = decompose_and_run(&mut cx, &goal, &runner, 2).unwrap();

    assert_eq!(
        report
            .subtasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        vec!["collect", "summarize"]
    );
    assert_eq!(
        report
            .outputs
            .iter()
            .map(|output| output.content.as_str())
            .collect::<Vec<_>>(),
        vec!["collected", "summarized"]
    );
    assert_eq!(report.budget_left, 0);
    assert_eq!(runner.calls(), 3);
}

#[test]
fn decompose_rejects_over_budget_plan_before_execution() {
    let runner = ScriptedRunner::new([response(vec![Expr::List(vec![
        task_expr("a", "A"),
        task_expr("b", "B"),
    ])])]);
    let mut cx = eval_cx();
    let goal = PlanningTask::new("goal", "Prepare answer");

    let err = decompose(&mut cx, &goal, &runner, 1).unwrap_err();

    assert!(format!("{err:?}").contains("over budget 1"));
    assert_eq!(runner.calls(), 1);
}

#[test]
fn decompose_propagates_runner_failure() {
    let runner = ScriptedRunner::new([ScriptStep::Error("planner unavailable")]);
    let mut cx = eval_cx();
    let goal = PlanningTask::new("goal", "Prepare answer");

    let err = decompose(&mut cx, &goal, &runner, 3).unwrap_err();

    assert!(format!("{err:?}").contains("planner unavailable"));
    assert_eq!(runner.calls(), 1);
}

#[test]
fn reflect_retries_once_and_tracks_budget() {
    let runner = ScriptedRunner::new([
        response(vec![Expr::Map(vec![
            (Expr::Symbol(Symbol::new("accept")), Expr::Bool(false)),
            (
                Expr::Symbol(Symbol::new("critique")),
                Expr::String("missing evidence".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("retry")),
                task_expr("retry", "Add evidence"),
            ),
        ])]),
        response(vec![Expr::String("fixed output".to_owned())]),
    ]);
    let mut cx = eval_cx();
    let output = PlanningOutput::new(PlanningTask::new("draft", "Draft answer"), "draft output");

    let reflection = reflect(&mut cx, &output, &runner, 1).unwrap();

    assert!(!reflection.accept);
    assert_eq!(reflection.critique, "missing evidence");
    assert_eq!(reflection.retry.as_ref().unwrap().prompt, "Add evidence");
    assert_eq!(
        reflection.retry_output.as_ref().unwrap().content,
        "fixed output"
    );
    assert_eq!(reflection.budget_left, 0);
    assert_eq!(runner.calls(), 2);
}

#[test]
fn reflect_fails_when_retry_budget_is_exhausted() {
    let runner = ScriptedRunner::new([response(vec![Expr::Map(vec![
        (Expr::Symbol(Symbol::new("accept")), Expr::Bool(false)),
        (
            Expr::Symbol(Symbol::new("retry")),
            task_expr("retry", "Try again"),
        ),
    ])])]);
    let mut cx = eval_cx();
    let output = PlanningOutput::new(PlanningTask::new("draft", "Draft answer"), "draft output");

    let err = reflect(&mut cx, &output, &runner, 0).unwrap_err();

    assert!(format!("{err:?}").contains("retry budget exhausted"));
    assert_eq!(runner.calls(), 1);
}

fn response(content: Vec<Expr>) -> ScriptStep {
    ScriptStep::Response(content)
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
