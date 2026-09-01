//! Process skill transport.
//!
//! Runs skills as external child-process commands, exchanging JSON over stdio
//! or streaming line-oriented text. Provides the [`ProcessSkillTransport`] and
//! its [`ProcessSkillSpec`]/[`ProcessSkillProtocol`] configuration. Gated
//! behind the `process` feature.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use sim_codec_json::{JsonProjectionMode, project_json_to_expr};
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, ShapeRef, Symbol, Value};
use sim_lib_agent_runner_core::{ModelEvent, ModelUsage};
use sim_lib_agent_runner_process::{BrokerProcessSpec, frame_stdout, run_broker_process};
use sim_lib_exec::{ProcessCancellation, ProgramRef, ProjectRootRef, SealedBindings};

use crate::{
    SkillCard, SkillEventSink, SkillPolicy, SkillRole, SkillTransport,
    skill_specific_call_capability, skill_transport_value,
};

const PROCESS_KIND: &str = "process";
const PROCESS_LABEL: &str = "skill/process";

/// Wire protocol a process skill uses to exchange data with its command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSkillProtocol {
    /// Exchange JSON on stdin and stdout.
    JsonStdio,
    /// Exchange plain text, optionally streaming output line by line.
    LineText,
}

/// Configuration for a single process-backed skill operation.
#[derive(Clone)]
pub struct ProcessSkillSpec {
    /// Stable string identifier for the skill.
    pub id: String,
    /// Operation name dispatched by the transport.
    pub operation: String,
    /// Human-readable title.
    pub title: String,
    /// Human-readable description.
    pub description: String,
    /// Template for the command line to run for each call.
    pub command_template: String,
    /// Protocol for exchanging data with the command.
    pub protocol: ProcessSkillProtocol,
    /// Shape contract for the call arguments.
    pub input_shape: ShapeRef,
    /// Shape contract for the call result.
    pub output_shape: ShapeRef,
    /// Roles the skill plays; defaults to tool when empty.
    pub roles: Vec<SkillRole>,
    /// Maximum time a single command may run before it is aborted.
    pub timeout: Duration,
    /// Maximum number of output bytes captured from the command.
    pub max_output_bytes: usize,
}

#[derive(Clone)]
struct ProcessSkillOperation {
    spec: ProcessSkillSpec,
}

/// Skill transport that runs each operation as an external process command.
pub struct ProcessSkillTransport {
    id: String,
    operations: BTreeMap<String, ProcessSkillOperation>,
    calls: AtomicUsize,
}

impl ProcessSkillTransport {
    /// Creates an empty process transport with the given `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            operations: BTreeMap::new(),
            calls: AtomicUsize::new(0),
        }
    }

    /// Registers `spec`, keyed by its operation name.
    pub fn insert(&mut self, spec: ProcessSkillSpec) {
        self.operations
            .insert(spec.operation.clone(), ProcessSkillOperation { spec });
    }

    /// Returns the number of calls dispatched through this transport.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Wraps this transport in an opaque skill transport value.
    pub fn value(self: Arc<Self>, cx: &mut Cx) -> Result<Value> {
        skill_transport_value(cx, self)
    }
}

impl SkillTransport for ProcessSkillTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        PROCESS_KIND
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        self.operations
            .values()
            .map(|operation| card_for_operation(&self.id, operation))
            .collect()
    }

    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        cx.require(&skill_process_capability())?;
        let operation = self
            .operations
            .get(&card.operation)
            .ok_or_else(|| Error::Eval(format!("process skill operation {} not found", card.id)))?
            .clone();
        self.calls.fetch_add(1, Ordering::SeqCst);
        match operation.spec.protocol {
            ProcessSkillProtocol::JsonStdio => call_json_stdio(cx, &operation, args),
            ProcessSkillProtocol::LineText => call_line_text(cx, card, &operation, args, events),
        }
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/process-health"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
            (
                Symbol::new("transport-kind"),
                cx.factory().string(PROCESS_KIND.to_owned())?,
            ),
            (
                Symbol::new("operations"),
                cx.factory().string(self.operations.len().to_string())?,
            ),
            (
                Symbol::new("calls"),
                cx.factory().string(self.call_count().to_string())?,
            ),
        ])
    }
}

/// Returns the capability required to run a process-backed skill.
pub fn skill_process_capability() -> CapabilityName {
    CapabilityName::new("skill.process")
}

fn card_for_operation(transport_id: &str, operation: &ProcessSkillOperation) -> Result<SkillCard> {
    let spec = &operation.spec;
    let mut roles = spec.roles.clone();
    if roles.is_empty() {
        roles.push(SkillRole::Tool);
    }
    Ok(SkillCard {
        id: spec.id.clone(),
        symbol: Symbol::qualified("skill", spec.id.clone()),
        aliases: vec![Symbol::qualified("process", spec.operation.clone())],
        origin: Symbol::new("process"),
        title: spec.title.clone(),
        description: spec.description.clone(),
        input_shape: spec.input_shape.clone(),
        output_shape: spec.output_shape.clone(),
        roles,
        capabilities: vec![
            skill_specific_call_capability(&spec.id),
            skill_process_capability(),
        ],
        policy: SkillPolicy::default(),
        transport_id: transport_id.to_owned(),
        transport_kind: PROCESS_KIND.to_owned(),
        operation: spec.operation.clone(),
    })
}

fn call_json_stdio(cx: &mut Cx, operation: &ProcessSkillOperation, args: Value) -> Result<Value> {
    let stdin = json_stdin(cx, args)?;
    let stdout = run_broker_process(
        cx,
        &command_spec(operation)?,
        stdin,
        &ProcessCancellation::default(),
    )?;
    let json: JsonValue = serde_json::from_slice(&stdout)
        .map_err(|err| Error::Eval(format!("{PROCESS_LABEL} returned invalid json: {err}")))?;
    cx.factory().expr(project_json_to_expr(
        &json,
        JsonProjectionMode::UntaggedInterop,
    ))
}

fn call_line_text(
    cx: &mut Cx,
    card: &SkillCard,
    operation: &ProcessSkillOperation,
    args: Value,
    events: Option<&mut dyn SkillEventSink>,
) -> Result<Value> {
    let stdin = text_stdin(cx, args)?;
    let stdout = if card.roles.contains(&SkillRole::Model) {
        match events {
            Some(sink) => stream_line_text(cx, card, operation, stdin, sink)?,
            None => run_broker_process(
                cx,
                &command_spec(operation)?,
                stdin,
                &ProcessCancellation::default(),
            )?,
        }
    } else {
        run_broker_process(
            cx,
            &command_spec(operation)?,
            stdin,
            &ProcessCancellation::default(),
        )?
    };
    let text = String::from_utf8(stdout)
        .map_err(|_| Error::Eval(format!("{PROCESS_LABEL} returned non-utf8 text")))?;
    cx.factory().string(text)
}

fn stream_line_text(
    cx: &mut Cx,
    card: &SkillCard,
    operation: &ProcessSkillOperation,
    stdin: Vec<u8>,
    sink: &mut dyn SkillEventSink,
) -> Result<Vec<u8>> {
    let span_id = Expr::String(card.operation.clone());
    emit_model_event(
        cx,
        sink,
        ModelEvent::start(card.symbol.clone(), card.id.clone(), span_id.clone()),
    )?;
    let mut line_count = 0_u64;
    let runner = card.symbol.clone();
    let model = card.id.clone();
    let stdout = run_broker_process(
        cx,
        &command_spec(operation)?,
        stdin,
        &ProcessCancellation::default(),
    )?;
    for line in frame_stdout(stdout.clone(), "lines")? {
        let line = std::str::from_utf8(&line)
            .map_err(|_| Error::Eval(format!("{PROCESS_LABEL} returned non-utf8 text")))?;
        let line = line.trim_end_matches(['\r', '\n']);
        line_count += 1;
        emit_model_event(
            cx,
            sink,
            ModelEvent::delta_text(runner.clone(), model.clone(), span_id.clone(), line),
        )?;
    }
    emit_model_event(
        cx,
        sink,
        ModelEvent::usage(
            runner,
            model,
            span_id,
            ModelUsage {
                output_tokens: Some(line_count),
                ..ModelUsage::default()
            },
        ),
    )?;
    Ok(stdout)
}

fn emit_model_event(cx: &mut Cx, sink: &mut dyn SkillEventSink, event: ModelEvent) -> Result<()> {
    let expr: Expr = event.into();
    let value = cx.factory().expr(expr)?;
    sink.emit(cx, value)
}

fn command_spec(operation: &ProcessSkillOperation) -> Result<BrokerProcessSpec> {
    BrokerProcessSpec::new(
        ProgramRef::new(operation.spec.command_template.clone())?,
        Vec::new(),
        ProjectRootRef::new("skill-process")?,
        SealedBindings::empty(),
        Vec::new(),
        PROCESS_LABEL,
        operation.spec.timeout,
        operation.spec.max_output_bytes,
    )
}

fn json_stdin(cx: &mut Cx, args: Value) -> Result<Vec<u8>> {
    let expr = args.object().as_expr(cx)?;
    serde_json::to_vec(&expr_to_json(&expr)).map_err(|err| Error::HostError(err.to_string()))
}

fn text_stdin(cx: &mut Cx, args: Value) -> Result<Vec<u8>> {
    let expr = args.object().as_expr(cx)?;
    let text = match expr {
        Expr::List(items) if items.len() == 1 => expr_text(&items[0]),
        other => expr_text(&other),
    };
    Ok(text.into_bytes())
}

// Intentionally-divergent untagged Expr->JSON projection: this module's wire
// form differs from sim_codec_json::project_expr_to_json and stays local because
// process skill payloads need this untagged shape.
fn expr_to_json(expr: &Expr) -> JsonValue {
    match expr {
        Expr::Nil => JsonValue::Null,
        Expr::Bool(value) => JsonValue::Bool(*value),
        Expr::Number(number) => number_json(&number.canonical),
        Expr::Symbol(symbol) | Expr::Local(symbol) => JsonValue::String(symbol.to_string()),
        Expr::String(text) => JsonValue::String(text.clone()),
        Expr::Bytes(bytes) => JsonValue::Array(
            bytes
                .iter()
                .map(|byte| JsonValue::Number(JsonNumber::from(*byte)))
                .collect(),
        ),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            JsonValue::Array(items.iter().map(expr_to_json).collect())
        }
        Expr::Map(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| (json_key(key), expr_to_json(value)))
                .collect(),
        ),
        Expr::Call { operator, args } => tagged_json(
            "call",
            vec![
                ("operator", expr_to_json(operator)),
                (
                    "args",
                    JsonValue::Array(args.iter().map(expr_to_json).collect()),
                ),
            ],
        ),
        Expr::Infix {
            operator,
            left,
            right,
        } => tagged_json(
            "infix",
            vec![
                ("operator", JsonValue::String(operator.to_string())),
                ("left", expr_to_json(left)),
                ("right", expr_to_json(right)),
            ],
        ),
        Expr::Prefix { operator, arg } => tagged_json(
            "prefix",
            vec![
                ("operator", JsonValue::String(operator.to_string())),
                ("arg", expr_to_json(arg)),
            ],
        ),
        Expr::Postfix { operator, arg } => tagged_json(
            "postfix",
            vec![
                ("operator", JsonValue::String(operator.to_string())),
                ("arg", expr_to_json(arg)),
            ],
        ),
        Expr::Quote { expr, .. } | Expr::Annotated { expr, .. } => expr_to_json(expr),
        Expr::Extension { tag, payload } => tagged_json(
            "extension",
            vec![
                ("tag", JsonValue::String(tag.to_string())),
                ("payload", expr_to_json(payload)),
            ],
        ),
    }
}

fn number_json(canonical: &str) -> JsonValue {
    if let Ok(value) = canonical.parse::<i64>() {
        return JsonValue::Number(JsonNumber::from(value));
    }
    canonical
        .parse::<f64>()
        .ok()
        .and_then(JsonNumber::from_f64)
        .map(JsonValue::Number)
        .unwrap_or_else(|| JsonValue::String(canonical.to_owned()))
}

fn tagged_json(tag: &str, fields: Vec<(&str, JsonValue)>) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert("$expr".to_owned(), JsonValue::String(tag.to_owned()));
    for (key, value) in fields {
        object.insert(key.to_owned(), value);
    }
    JsonValue::Object(object)
}

fn json_key(expr: &Expr) -> String {
    match expr {
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.name.to_string(),
        Expr::String(text) => text.clone(),
        other => expr_text(other),
    }
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => String::new(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        Expr::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().map(expr_text).collect::<Vec<_>>().join(" ")
        }
        Expr::Map(entries) => entries
            .iter()
            .map(|(key, value)| format!("{}: {}", expr_text(key), expr_text(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        Expr::Call { operator, args } => {
            let mut parts = vec![expr_text(operator)];
            parts.extend(args.iter().map(expr_text));
            parts.join(" ")
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => format!("{} {} {}", expr_text(left), operator, expr_text(right)),
        Expr::Prefix { operator, arg } => format!("{operator} {}", expr_text(arg)),
        Expr::Postfix { operator, arg } => format!("{} {operator}", expr_text(arg)),
        Expr::Quote { expr, .. } | Expr::Annotated { expr, .. } => expr_text(expr),
        Expr::Extension { tag, payload } => format!("{tag} {}", expr_text(payload)),
    }
}

#[cfg(test)]
mod tests;
