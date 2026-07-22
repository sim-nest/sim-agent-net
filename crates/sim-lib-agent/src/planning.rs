use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelRunner};
use sim_value::access::{entry_required_bool_any, entry_required_str_any, entry_required_sym_any};

/// Unit of work produced by planning combinators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanningTask {
    /// Stable task identifier.
    pub id: String,
    /// Prompt that drives the task.
    pub prompt: String,
}

impl PlanningTask {
    /// Builds a planning task from an id and prompt.
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            prompt: prompt.into(),
        }
    }
}

/// Output captured after running one planning task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanningOutput {
    /// Task that produced this output.
    pub task: PlanningTask,
    /// Text content captured from the run.
    pub content: String,
}

impl PlanningOutput {
    /// Builds an output from a task and its captured content.
    pub fn new(task: PlanningTask, content: impl Into<String>) -> Self {
        Self {
            task,
            content: content.into(),
        }
    }
}

/// Result of decomposing a goal and running the ordered subtasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decomposition {
    /// Original goal that was decomposed.
    pub goal: PlanningTask,
    /// Ordered subtasks produced by decomposition.
    pub subtasks: Vec<PlanningTask>,
    /// Output captured for each executed subtask.
    pub outputs: Vec<PlanningOutput>,
    /// Remaining step budget after execution.
    pub budget_left: u32,
}

/// Critique result, including a bounded retry when the runner asks for one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reflection {
    /// Whether the runner accepted the output.
    pub accept: bool,
    /// Critique text explaining the verdict.
    pub critique: String,
    /// Retry task requested by the runner, when any.
    pub retry: Option<PlanningTask>,
    /// Output captured from the executed retry, when one ran.
    pub retry_output: Option<PlanningOutput>,
    /// Remaining retry budget after reflection.
    pub budget_left: u32,
}

/// Asks a runner to split a goal into deterministic ordered subtasks.
pub fn decompose(
    cx: &mut Cx,
    goal: &PlanningTask,
    runner: &dyn ModelRunner,
    max_steps: u32,
) -> Result<Vec<PlanningTask>> {
    if max_steps == 0 {
        return Err(Error::Eval(
            "decompose budget exhausted before planning".to_owned(),
        ));
    }
    let response = runner.infer(
        cx,
        ModelRequest::new(
            operation_expr("decompose", vec![(Symbol::new("goal"), task_expr(goal))]),
            Vec::new(),
        ),
    )?;
    let subtasks = parse_tasks(&response, &goal.id)?;
    if subtasks.is_empty() {
        return Err(Error::Eval(
            "decompose runner returned no subtasks".to_owned(),
        ));
    }
    if subtasks.len() as u32 > max_steps {
        return Err(Error::Eval(format!(
            "decompose produced {} subtasks over budget {max_steps}",
            subtasks.len()
        )));
    }
    Ok(subtasks)
}

/// Decomposes a goal, then executes each returned subtask in order.
pub fn decompose_and_run(
    cx: &mut Cx,
    goal: &PlanningTask,
    runner: &dyn ModelRunner,
    max_steps: u32,
) -> Result<Decomposition> {
    let subtasks = decompose(cx, goal, runner, max_steps)?;
    let mut outputs = Vec::with_capacity(subtasks.len());
    for task in &subtasks {
        outputs.push(run_task(cx, task, runner)?);
    }
    Ok(Decomposition {
        goal: goal.clone(),
        budget_left: max_steps.saturating_sub(outputs.len() as u32),
        subtasks,
        outputs,
    })
}

/// Critiques an output and runs at most one retry requested by the runner.
pub fn reflect(
    cx: &mut Cx,
    output: &PlanningOutput,
    runner: &dyn ModelRunner,
    retry_budget: u32,
) -> Result<Reflection> {
    let response = runner.infer(
        cx,
        ModelRequest::new(
            operation_expr(
                "reflect",
                vec![(Symbol::new("output"), output_expr(output))],
            ),
            Vec::new(),
        ),
    )?;
    let mut reflection = parse_reflection(&response, retry_budget)?;
    if !reflection.accept && reflection.retry.is_some() {
        if retry_budget == 0 {
            return Err(Error::Eval(
                "reflect retry budget exhausted before re-run".to_owned(),
            ));
        }
        let retry = reflection.retry.clone().expect("retry checked above");
        reflection.retry_output = Some(run_task(cx, &retry, runner)?);
        reflection.budget_left = retry_budget - 1;
    }
    Ok(reflection)
}

fn run_task(cx: &mut Cx, task: &PlanningTask, runner: &dyn ModelRunner) -> Result<PlanningOutput> {
    let response = runner.infer(
        cx,
        ModelRequest::new(
            operation_expr("execute", vec![(Symbol::new("task"), task_expr(task))]),
            Vec::new(),
        ),
    )?;
    Ok(PlanningOutput::new(task.clone(), response_text(&response)))
}

fn parse_tasks(response: &ModelResponse, goal_id: &str) -> Result<Vec<PlanningTask>> {
    let payloads = response_payloads(response);
    let mut tasks = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        match payload {
            Expr::List(items) | Expr::Vector(items) => {
                for (item_index, item) in items.iter().enumerate() {
                    tasks.push(parse_task(item, &step_id(goal_id, item_index + 1))?);
                }
            }
            Expr::String(text) => {
                tasks.extend(parse_text_tasks(goal_id, text));
            }
            Expr::Map(entries) => {
                if let Some(text) = text_part(entries) {
                    tasks.extend(parse_text_tasks(goal_id, &text));
                } else {
                    tasks.push(parse_task(payload, &step_id(goal_id, index + 1))?);
                }
            }
            expr => tasks.push(parse_task(expr, &step_id(goal_id, index + 1))?),
        }
    }
    Ok(tasks)
}

fn parse_reflection(response: &ModelResponse, retry_budget: u32) -> Result<Reflection> {
    let payloads = response_payloads(response);
    if let Some(Expr::Map(entries)) = payloads.first() {
        let accept = bool_field(entries, "accept").unwrap_or(false);
        let critique = string_field(entries, "critique")
            .or_else(|| string_field(entries, "reason"))
            .unwrap_or_else(|| response_text(response));
        let retry = expr_field(entries, "retry")
            .or_else(|| expr_field(entries, "retry-task"))
            .filter(|expr| !matches!(expr, Expr::Nil))
            .map(|expr| parse_task(expr, "retry-1"))
            .transpose()?;
        return Ok(Reflection {
            accept,
            critique,
            retry,
            retry_output: None,
            budget_left: retry_budget,
        });
    }

    let text = response_text(response);
    let lower = text.to_ascii_lowercase();
    let accept = lower.starts_with("accept") || lower.contains("accept: true");
    let retry = retry_text(&text).map(|prompt| PlanningTask::new("retry-1", prompt));
    Ok(Reflection {
        accept,
        critique: text,
        retry,
        retry_output: None,
        budget_left: retry_budget,
    })
}

fn parse_task(expr: &Expr, default_id: &str) -> Result<PlanningTask> {
    match expr {
        Expr::Map(entries) => {
            let id = string_field(entries, "id").unwrap_or_else(|| default_id.to_owned());
            let prompt = string_field(entries, "prompt")
                .or_else(|| string_field(entries, "instruction"))
                .or_else(|| string_field(entries, "task"))
                .ok_or_else(|| {
                    Error::Eval(format!(
                        "planning task {id} requires prompt, instruction, or task"
                    ))
                })?;
            Ok(PlanningTask::new(id, prompt))
        }
        Expr::String(text) => Ok(PlanningTask::new(default_id.to_owned(), text.clone())),
        expr => Ok(PlanningTask::new(default_id.to_owned(), expr_text(expr))),
    }
}

fn parse_text_tasks(goal_id: &str, text: &str) -> Vec<PlanningTask> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(index, line)| {
            PlanningTask::new(step_id(goal_id, index + 1), clean_list_marker(line))
        })
        .collect()
}

fn operation_expr(name: &str, fields: Vec<(Symbol, Expr)>) -> Expr {
    let mut entries = vec![(
        Expr::Symbol(Symbol::new("op")),
        Expr::Symbol(Symbol::new(name)),
    )];
    entries.extend(
        fields
            .into_iter()
            .map(|(key, value)| (Expr::Symbol(key), value)),
    );
    Expr::Map(entries)
}

fn task_expr(task: &PlanningTask) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("id")),
            Expr::String(task.id.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("prompt")),
            Expr::String(task.prompt.clone()),
        ),
    ])
}

fn output_expr(output: &PlanningOutput) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("task")), task_expr(&output.task)),
        (
            Expr::Symbol(Symbol::new("content")),
            Expr::String(output.content.clone()),
        ),
    ])
}

fn response_payloads(response: &ModelResponse) -> Vec<Expr> {
    if response.content.len() == 1
        && let Expr::Map(entries) = &response.content[0]
        && let Some(text) = text_part(entries)
    {
        return vec![Expr::String(text)];
    }
    response.content.clone()
}

fn response_text(response: &ModelResponse) -> String {
    response
        .content
        .iter()
        .map(expr_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        Expr::Map(entries) => text_part(entries).unwrap_or_else(|| format!("{expr:?}")),
        _ => format!("{expr:?}"),
    }
}

fn text_part(entries: &[(Expr, Expr)]) -> Option<String> {
    match string_field(entries, "text") {
        Some(text) if matches!(symbol_field(entries, "type").as_deref(), Some("text")) => {
            Some(text)
        }
        _ => None,
    }
}

// These readers match a field name namespace-agnostically via the shared
// `sim_value::access` `_any` substrate (bare-symbol OR string key). `string_field`
// keeps String|Symbol value coercion behind thin Option adapters.
fn string_field(entries: &[(Expr, Expr)], field: &str) -> Option<String> {
    entry_required_str_any(entries, field, "string")
        .ok()
        .map(str::to_owned)
        .or_else(|| {
            entry_required_sym_any(entries, field, "symbol")
                .ok()
                .map(Symbol::to_string)
        })
}

fn symbol_field(entries: &[(Expr, Expr)], field: &str) -> Option<String> {
    entry_required_sym_any(entries, field, "symbol")
        .ok()
        .map(Symbol::to_string)
}

fn bool_field(entries: &[(Expr, Expr)], field: &str) -> Option<bool> {
    entry_required_bool_any(entries, field, "bool").ok()
}

// Raw any-namespace field borrow used by `parse_reflection` for the untyped
// `retry`/`retry-task` payload (which is matched then parsed as a task, not
// coerced to a scalar).
fn expr_field<'a>(entries: &'a [(Expr, Expr)], field: &str) -> Option<&'a Expr> {
    entries.iter().find_map(|(key, value)| {
        if matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == field) {
            Some(value)
        } else {
            None
        }
    })
}

fn retry_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    lower.find("retry:").map(|index| {
        text[index + "retry:".len()..]
            .trim()
            .trim_matches('"')
            .to_owned()
    })
}

fn clean_list_marker(line: &str) -> String {
    let trimmed = line.trim_start_matches(['-', '*', ' ']);
    let without_number = trimmed
        .split_once('.')
        .and_then(|(head, tail)| head.chars().all(|ch| ch.is_ascii_digit()).then_some(tail))
        .unwrap_or(trimmed);
    without_number.trim().to_owned()
}

fn step_id(goal_id: &str, one_based_index: usize) -> String {
    format!("{goal_id}.{one_based_index}")
}

#[cfg(test)]
mod field_reader_tests {
    use super::{bool_field, string_field, symbol_field};
    use sim_kernel::{Expr, Symbol};

    fn entry(key: Expr, value: Expr) -> (Expr, Expr) {
        (key, value)
    }

    // The readers deliberately match a field name namespace-agnostically via the
    // shared `entry_required_*_any` substrate: a bare-symbol key OR a string key
    // resolves.
    #[test]
    fn readers_match_bare_symbol_and_string_keys() {
        let bare = [entry(
            Expr::Symbol(Symbol::new("text")),
            Expr::String("hi".to_owned()),
        )];
        assert_eq!(string_field(&bare, "text"), Some("hi".to_owned()));

        let string_key = [entry(
            Expr::String("type".to_owned()),
            Expr::Symbol(Symbol::new("text")),
        )];
        assert_eq!(symbol_field(&string_key, "type"), Some("text".to_owned()));
    }

    // `string_field` coerces both String and Symbol values to an owned String;
    // `symbol_field` reads only symbols; `bool_field` reads only bools.
    #[test]
    fn readers_preserve_value_coercion() {
        let entries = [
            entry(
                Expr::Symbol(Symbol::new("text")),
                Expr::Symbol(Symbol::new("as-symbol")),
            ),
            entry(
                Expr::Symbol(Symbol::new("type")),
                Expr::Symbol(Symbol::new("text")),
            ),
            entry(Expr::Symbol(Symbol::new("flag")), Expr::Bool(true)),
        ];

        assert_eq!(string_field(&entries, "text"), Some("as-symbol".to_owned()));
        assert_eq!(symbol_field(&entries, "type"), Some("text".to_owned()));
        assert_eq!(bool_field(&entries, "flag"), Some(true));

        // Wrong-typed or absent fields read as None.
        assert_eq!(symbol_field(&entries, "flag"), None);
        assert_eq!(bool_field(&entries, "missing"), None);
    }
}
