//! Bounded CLI entrypoint for conduct discovery and durable run control.

use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
};

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Export, Expr, Linker, LoadCx,
    NumberLiteral, Object, ObjectCompat, Result, Symbol, Value,
};

const AGENT_CLI_VERB: &str = "agent";
const MAX_IDENTIFIER_LEN: usize = 96;

#[derive(Clone)]
struct CliRun {
    conduct: String,
    status: &'static str,
    sequence: u64,
    report: Expr,
}

static CLI_RUNS: LazyLock<Mutex<BTreeMap<String, CliRun>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn cli_main_symbol() -> Symbol {
    Symbol::qualified("cli", "main/agent")
}

pub(crate) fn agent_cli_exports() -> Vec<Export> {
    vec![Export::Function {
        symbol: cli_main_symbol(),
        function_id: None,
    }]
}

pub(crate) fn register_agent_cli(cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
    linker.function_value(
        cli_main_symbol(),
        cx.factory().opaque(Arc::new(AgentCliEntrypoint))?,
    )?;
    Ok(())
}

#[derive(Clone)]
struct AgentCliEntrypoint;

impl Object for AgentCliEntrypoint {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function cli/main/agent>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for AgentCliEntrypoint {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for AgentCliEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        verify_cli_envelope(cx, &args, AGENT_CLI_VERB)?;
        let envelope = &args.values()[0];
        let payload = envelope_args(cx, envelope)?;
        let command_args = payload[1..]
            .iter()
            .filter(|arg| {
                !matches!(
                    arg.as_str(),
                    "--json" | "--text" | "--format=json" | "--format=lisp" | "--format=text"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let output = dispatch_agent_cli(cx, &command_args)?;
        cx.factory().expr(output)
    }
}

pub(crate) fn dispatch_agent_cli(cx: &mut Cx, args: &[String]) -> Result<Expr> {
    match args {
        [group, command, rest @ ..] if group == "conduct" => {
            dispatch_conduct_cli(cx, command, rest)
        }
        [group, command, rest @ ..] if group == "run" => dispatch_run_cli(command, rest),
        _ => Err(cli_usage()),
    }
}

fn dispatch_conduct_cli(cx: &mut Cx, command: &str, args: &[String]) -> Result<Expr> {
    match (command, args) {
        ("list", []) => Ok(map(vec![
            ("kind", symbol("agent.cli", "conduct-list")),
            (
                "conducts",
                Expr::List(
                    sim_lib_agent_conduct::agent_conduct_catalog_sources()
                        .iter()
                        .map(|source| Expr::Symbol(Symbol::new(source.id)))
                        .collect(),
                ),
            ),
        ])),
        ("show" | "explain", [id]) => {
            validate_identifier(id)?;
            let conduct = catalog_conduct(cx, id)?;
            let topology = if command == "show" {
                conduct.reflect(cx)
            } else {
                conduct.diagram(cx)
            };
            Ok(map(vec![
                ("kind", symbol("agent.cli", command)),
                ("id", Expr::Symbol(Symbol::new(id.as_str()))),
                ("topology", topology),
                (
                    "step-cards",
                    Expr::List(
                        conduct
                            .step_cards
                            .iter()
                            .map(|card| Expr::Symbol(card.step_id.clone()))
                            .collect(),
                    ),
                ),
                (
                    "roles",
                    Expr::List(
                        conduct
                            .required_roles
                            .iter()
                            .cloned()
                            .map(Expr::Symbol)
                            .collect(),
                    ),
                ),
                (
                    "authority",
                    Expr::List(
                        conduct
                            .capabilities
                            .iter()
                            .map(|capability| Expr::Symbol(Symbol::new(capability.as_str())))
                            .collect(),
                    ),
                ),
                ("domain-budget", budget_expr(&conduct.topology.graph)),
                (
                    "result-shape",
                    sim_lib_agent_conduct::completed_or_stopped_shape(),
                ),
                ("replay-policy", symbol("agent.replay", "recorded")),
            ]))
        }
        ("run", [id, run_id]) => start_cli_run(cx, id, run_id),
        _ => Err(cli_usage()),
    }
}

fn dispatch_run_cli(command: &str, args: &[String]) -> Result<Expr> {
    match (command, args) {
        ("inspect", [run_id]) => with_run(run_id, |run| run_expr(run_id, run)),
        ("suspend", [run_id]) => mutate_run(run_id, |run| {
            run.status = "suspended";
            run.sequence += 1;
        }),
        ("resume", [run_id]) => mutate_run(run_id, |run| {
            run.status = "running";
            run.sequence += 1;
        }),
        ("replay", [run_id]) => with_run(run_id, |run| {
            Ok(map(vec![
                ("kind", symbol("agent.cli", "effect-free-replay")),
                ("run-id", Expr::Symbol(Symbol::new(run_id.as_str()))),
                ("live-effects", Expr::Bool(false)),
                ("report", run.report.clone()),
            ]))
        }),
        ("fork", [run_id, child_id]) => fork_run(run_id, child_id),
        _ => Err(cli_usage()),
    }
}

fn start_cli_run(cx: &mut Cx, conduct_id: &str, run_id: &str) -> Result<Expr> {
    validate_identifier(conduct_id)?;
    validate_identifier(run_id)?;
    let conduct = catalog_conduct(cx, conduct_id)?;
    let topology_report = conduct.diagram(cx);
    let report = map(vec![
        ("graph-fingerprint", Expr::String(conduct.graph_fingerprint)),
        ("topology-report", topology_report),
    ]);
    let run = CliRun {
        conduct: conduct_id.to_owned(),
        status: "running",
        sequence: 0,
        report,
    };
    let mut runs = CLI_RUNS
        .lock()
        .map_err(|_| Error::Eval("agent CLI run registry is unavailable".into()))?;
    if runs.insert(run_id.to_owned(), run.clone()).is_some() {
        return Err(Error::Eval(format!("agent run {run_id} already exists")));
    }
    run_expr(run_id, &run)
}

fn catalog_conduct(cx: &mut Cx, id: &str) -> Result<sim_lib_agent_conduct::AgentConduct> {
    let source = sim_lib_agent_conduct::agent_conduct_catalog_sources()
        .iter()
        .find(|source| source.id == id)
        .ok_or_else(|| Error::Eval(format!("unknown agent conduct {id}")))?;
    let package = sim_lib_topology::parse_package(source.source)?;
    let _ = cx;
    sim_lib_agent_conduct::validate_agent_conduct(package, &crate::standard_step_cards())
}

fn budget_expr(graph: &sim_lib_topology::Graph) -> Expr {
    map(vec![
        ("max-steps", number(graph.budget.max_steps)),
        ("max-node-visits", number(graph.budget.max_node_visits)),
        ("max-edge-visits", number(graph.budget.max_edge_visits)),
    ])
}

fn with_run(run_id: &str, f: impl FnOnce(&CliRun) -> Result<Expr>) -> Result<Expr> {
    validate_identifier(run_id)?;
    let runs = CLI_RUNS
        .lock()
        .map_err(|_| Error::Eval("agent CLI run registry is unavailable".into()))?;
    let run = runs
        .get(run_id)
        .ok_or_else(|| Error::Eval(format!("unknown agent run {run_id}")))?;
    f(run)
}

fn mutate_run(run_id: &str, mutate: impl FnOnce(&mut CliRun)) -> Result<Expr> {
    validate_identifier(run_id)?;
    let mut runs = CLI_RUNS
        .lock()
        .map_err(|_| Error::Eval("agent CLI run registry is unavailable".into()))?;
    let run = runs
        .get_mut(run_id)
        .ok_or_else(|| Error::Eval(format!("unknown agent run {run_id}")))?;
    mutate(run);
    run_expr(run_id, run)
}

fn fork_run(parent_id: &str, child_id: &str) -> Result<Expr> {
    validate_identifier(parent_id)?;
    validate_identifier(child_id)?;
    let mut runs = CLI_RUNS
        .lock()
        .map_err(|_| Error::Eval("agent CLI run registry is unavailable".into()))?;
    let parent = runs
        .get(parent_id)
        .cloned()
        .ok_or_else(|| Error::Eval(format!("unknown agent run {parent_id}")))?;
    if runs.contains_key(child_id) {
        return Err(Error::Eval(format!("agent run {child_id} already exists")));
    }
    let mut child = parent.clone();
    child.sequence = 0;
    child.status = "suspended";
    runs.insert(child_id.to_owned(), child.clone());
    Ok(map(vec![
        ("kind", symbol("agent.cli", "fork")),
        ("parent-run-id", Expr::Symbol(Symbol::new(parent_id))),
        ("child", run_expr(child_id, &child)?),
    ]))
}

fn run_expr(run_id: &str, run: &CliRun) -> Result<Expr> {
    Ok(map(vec![
        ("kind", symbol("agent.cli", "run")),
        ("run-id", Expr::Symbol(Symbol::new(run_id))),
        ("conduct", Expr::Symbol(Symbol::new(run.conduct.as_str()))),
        ("status", symbol("agent.run", run.status)),
        ("sequence", number(run.sequence)),
        ("report", run.report.clone()),
    ]))
}

fn map(entries: Vec<(&str, Expr)>) -> Expr {
    Expr::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Expr::Symbol(Symbol::new(key)), value))
            .collect(),
    )
}

fn symbol(namespace: &str, name: &str) -> Expr {
    Expr::Symbol(Symbol::qualified(namespace, name))
}

fn number(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "i64"),
        canonical: value.to_string(),
    })
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_LEN
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/'))
    {
        return Err(Error::Eval(
            "agent CLI identifier is invalid or exceeds its bound".into(),
        ));
    }
    Ok(())
}

fn cli_usage() -> Error {
    Error::Eval(
        "usage: agent conduct <list|show|explain|run> ... | agent run <inspect|suspend|resume|replay|fork> ..."
            .into(),
    )
}

fn verify_cli_envelope(cx: &mut Cx, args: &Args, verb: &str) -> Result<()> {
    let envelope = args
        .values()
        .first()
        .ok_or_else(|| Error::Eval(format!("cli/main/{verb} expects a CLI envelope")))?;
    let envelope_verb = envelope_string_field(cx, envelope, "verb")?;
    if envelope_verb != verb {
        return Err(Error::Eval(format!(
            "cli/main/{verb} received verb {envelope_verb}"
        )));
    }
    let payload_args = envelope_args(cx, envelope)?;
    if payload_args.first().map(String::as_str) != Some(verb) {
        return Err(Error::Eval(format!(
            "cli/main/{verb} expects the first payload argument to be {verb}"
        )));
    }
    Ok(())
}

fn envelope_string_field(cx: &mut Cx, envelope: &Value, field: &str) -> Result<String> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    match table.get(cx, Symbol::new(field))?.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        Expr::Nil => Err(Error::Eval(format!("CLI envelope field {field} is nil"))),
        other => Err(Error::Eval(format!(
            "CLI envelope field {field} is not a string: {other:?}"
        ))),
    }
}

fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    let value = table.get(cx, Symbol::new("args"))?;
    let Some(list) = value.object().as_list() else {
        return Err(Error::Eval(
            "CLI envelope field args is not a list".to_owned(),
        ));
    };
    list.to_vec(cx, Some(64))?
        .into_iter()
        .map(|value| match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            other => Err(Error::Eval(format!(
                "CLI payload argument is not a string: {other:?}"
            ))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
        let Expr::Map(entries) = expr else {
            panic!("expected map")
        };
        entries
            .iter()
            .find_map(|(key, value)| {
                matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == name).then_some(value)
            })
            .expect("field")
    }

    #[test]
    fn conduct_cli_lists_and_reuses_topology_reflection() {
        let mut cx = sim_kernel::testing::bare_cx();
        let listed = dispatch_agent_cli(&mut cx, &["conduct".into(), "list".into()]).unwrap();
        assert!(matches!(field(&listed, "conducts"), Expr::List(items) if items.len() == 7));

        let shown = dispatch_agent_cli(
            &mut cx,
            &["conduct".into(), "show".into(), "agent/default-v1".into()],
        )
        .unwrap();
        assert!(matches!(field(&shown, "topology"), Expr::Map(_)));
        assert!(matches!(field(&shown, "step-cards"), Expr::List(_)));
        assert_eq!(
            field(&shown, "result-shape"),
            &Expr::Symbol(Symbol::qualified("agent", "RunFrame"))
        );
    }

    #[test]
    fn run_lifecycle_is_bounded_effect_free_and_forkable() {
        let mut cx = sim_kernel::testing::bare_cx();
        cx.grant(sim_lib_topology::topology_reflect_capability());
        cx.grant(sim_lib_topology::topology_run_capability());
        let run_id = "cli-lifecycle-spec";
        dispatch_agent_cli(
            &mut cx,
            &[
                "conduct".into(),
                "run".into(),
                "agent/default-v1".into(),
                run_id.into(),
            ],
        )
        .unwrap();
        let suspended =
            dispatch_agent_cli(&mut cx, &["run".into(), "suspend".into(), run_id.into()]).unwrap();
        assert_eq!(
            field(&suspended, "status"),
            &Expr::Symbol(Symbol::qualified("agent.run", "suspended"))
        );
        let replayed =
            dispatch_agent_cli(&mut cx, &["run".into(), "replay".into(), run_id.into()]).unwrap();
        assert_eq!(field(&replayed, "live-effects"), &Expr::Bool(false));
        let forked = dispatch_agent_cli(
            &mut cx,
            &[
                "run".into(),
                "fork".into(),
                run_id.into(),
                "cli-lifecycle-child".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            field(&forked, "parent-run-id"),
            &Expr::Symbol(Symbol::new(run_id))
        );
    }
}
