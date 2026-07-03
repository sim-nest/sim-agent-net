use crate::{Tool, stringish_from_value, symbol_from_value};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{EvalSite, FrameEnvelope, FrameKind, ServerFrame, parse_duration};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CURRENT_TASK_ID: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct TaskIdGuard {
    previous: Option<String>,
}

impl Drop for TaskIdGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT_TASK_ID.with(|slot| {
            slot.replace(previous);
        });
    }
}

fn trace_registry() -> &'static Mutex<BTreeMap<String, Vec<Expr>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Vec<Expr>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn ensure_task_id(frame: &mut ServerFrame) -> String {
    if let Some(task_id) = current_task_id() {
        return task_id;
    }
    let next = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
    if frame.msg_id.is_none() {
        frame.msg_id = Some(next);
    }
    format!("task-{next}")
}

pub(crate) fn with_task_id<T>(task_id: String, f: impl FnOnce() -> Result<T>) -> Result<T> {
    CURRENT_TASK_ID.with(|slot| {
        let previous = slot.replace(Some(task_id));
        let _guard = TaskIdGuard { previous };
        f()
    })
}

pub(crate) fn record_trace_entry(
    cx: &mut Cx,
    recorders: &[Value],
    frame: &ServerFrame,
    stage: &str,
    phase: &str,
    tool: Option<&Symbol>,
) -> Result<()> {
    if recorders.is_empty() {
        return Ok(());
    }
    let entry = trace_entry_expr(cx, frame, stage, phase, tool)?;
    for recorder in recorders {
        let site = crate::agents::site_from_value(recorder)?;
        let notify = trace_notify_frame(cx, &site, frame, &entry)?;
        let _ = site.answer(cx, notify)?;
    }
    Ok(())
}

pub(crate) fn remember_recorded_trace(entry: &Expr) -> Result<()> {
    if !is_trace_entry(entry) {
        return Ok(());
    }
    append_global_trace(entry)
}

pub(crate) fn trace_entries(task_id: &str) -> Result<Vec<Expr>> {
    let traces = trace_registry()
        .lock()
        .map_err(|_| Error::PoisonedLock("trace registry"))?;
    Ok(traces.get(task_id).cloned().unwrap_or_default())
}

pub(crate) fn trace_timestamp_ms(entry: &Expr) -> Option<u64> {
    trace_field(entry, "timestamp-ms").and_then(expr_u64)
}

pub(crate) fn trace_role_matches(entry: &Expr, role: &Symbol) -> bool {
    matches!(
        trace_field(entry, "role"),
        Some(Expr::Symbol(found)) if found == role
    )
}

pub(crate) fn trace_tool_matches(entry: &Expr, tool: &Symbol) -> bool {
    matches!(
        trace_field(entry, "tool"),
        Some(Expr::Symbol(found)) if found == tool
    )
}

pub(crate) fn parse_since_cutoff(cx: &mut Cx, value: Option<&Value>) -> Result<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let duration = parse_duration(&Expr::String(stringish_from_value(
        cx,
        value.clone(),
        "agent/audit :since expects a duration string",
    )?))?;
    let now = now_millis();
    let span = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    Ok(Some(now.saturating_sub(span)))
}

pub(crate) fn audit_role_filter(
    cx: &mut Cx,
    value: Option<&Value>,
    label: &'static str,
) -> Result<Option<Symbol>> {
    value
        .map(|value| symbol_from_value(cx, value.clone(), label))
        .transpose()
}

fn trace_entry_expr(
    cx: &mut Cx,
    frame: &ServerFrame,
    stage: &str,
    phase: &str,
    tool: Option<&Symbol>,
) -> Result<Expr> {
    let payload = frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?;
    let task_id = current_task_id()
        .or_else(|| frame.msg_id.map(|id| format!("task-{id}")))
        .or_else(|| frame.correlate.map(|id| format!("task-{id}")))
        .unwrap_or_else(|| "task-0".to_owned());
    crate::model_privacy::redact_trace_entry(Expr::Map(vec![
        (Expr::Symbol(Symbol::new("trace-entry")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("timestamp-ms")),
            Expr::String(now_millis().to_string()),
        ),
        (Expr::Symbol(Symbol::new("task-id")), Expr::String(task_id)),
        (
            Expr::Symbol(Symbol::new("phase")),
            Expr::Symbol(Symbol::new(phase)),
        ),
        (
            Expr::Symbol(Symbol::new("stage")),
            Expr::String(stage.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("tool")),
            tool.cloned().map(Expr::Symbol).unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("kind")),
            Expr::Symbol(frame.kind.as_symbol()),
        ),
        (
            Expr::Symbol(Symbol::new("codec")),
            Expr::Symbol(frame.codec.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("role")),
            frame
                .envelope
                .role
                .clone()
                .map(Expr::Symbol)
                .unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("hop")),
            Expr::String(frame.envelope.hop.to_string()),
        ),
        (
            Expr::Symbol(Symbol::new("msg-id")),
            frame
                .msg_id
                .map(|id| Expr::String(id.to_string()))
                .unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("correlate")),
            frame
                .correlate
                .map(|id| Expr::String(id.to_string()))
                .unwrap_or(Expr::Nil),
        ),
        (
            Expr::Symbol(Symbol::new("trigger-source")),
            frame
                .envelope
                .trigger_source
                .clone()
                .map(Expr::Symbol)
                .unwrap_or(Expr::Nil),
        ),
        (Expr::Symbol(Symbol::new("payload")), payload),
    ]))
}

fn trace_notify_frame(
    cx: &mut Cx,
    site: &Arc<dyn EvalSite>,
    source: &ServerFrame,
    entry: &Expr,
) -> Result<ServerFrame> {
    let mut notify = ServerFrame::from_expr(
        cx,
        site.codecs()
            .first()
            .cloned()
            .unwrap_or_else(|| Symbol::qualified("codec", "binary")),
        FrameKind::Notify,
        entry,
        source.envelope.consistency,
        source.envelope.required_capabilities.clone(),
        source.envelope.trace,
    )?;
    notify.msg_id = source.msg_id;
    notify.correlate = source.correlate;
    notify.envelope = FrameEnvelope {
        deadline: source.envelope.deadline,
        consistency: source.envelope.consistency,
        trace: source.envelope.trace,
        required_capabilities: source.envelope.required_capabilities.clone(),
        reply_codec_hint: source.envelope.reply_codec_hint.clone(),
        role: source.envelope.role.clone(),
        hop: source.envelope.hop,
        trigger_source: source.envelope.trigger_source.clone(),
    };
    Ok(notify)
}

fn append_global_trace(entry: &Expr) -> Result<()> {
    let Some(Expr::String(task_id)) = trace_field(entry, "task-id") else {
        return Ok(());
    };
    let mut traces = trace_registry()
        .lock()
        .map_err(|_| Error::PoisonedLock("trace registry"))?;
    let bucket = traces.entry(task_id.clone()).or_default();
    if !bucket.iter().any(|existing| existing == entry) {
        bucket.push(entry.clone());
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn current_task_id() -> Option<String> {
    CURRENT_TASK_ID.with(|slot| slot.borrow().clone())
}

fn trace_field<'a>(entry: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = entry else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn is_trace_entry(entry: &Expr) -> bool {
    matches!(trace_field(entry, "trace-entry"), Some(Expr::Bool(true)))
}

fn expr_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::String(text) => text.parse().ok(),
        Expr::Number(number) => number.canonical.parse().ok(),
        _ => None,
    }
}

pub(crate) fn tool_symbol(value: &Value) -> Option<Symbol> {
    value
        .object()
        .downcast_ref::<Tool>()
        .map(|tool| tool.symbol.clone())
}
