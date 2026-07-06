use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
        mpsc::Receiver,
    },
    thread::JoinHandle,
    time::Duration,
};

use sim_citizen_derive::non_citizen;
use sim_kernel::{
    ClassRef, Cx, Error, Expr, Object, ReadPolicy, Result, Symbol, TrustLevel, Value,
    read_construct_capability, read_eval_capability,
};

use crate::{Server, ServerAddress, cron::CronMatcher, ensure_installed_codec};

mod runtime;
mod source_options;
mod sources;

use source_options::source_loopback;
use sources::{TriggerSource, build_network_source, source_capability};
#[cfg(test)]
use sources::{loopback_smtp_messages, queued_trigger_events};

static NEXT_TRIGGER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
enum TriggerDecoder {
    Codec(Symbol),
    Callable(Value),
}

struct TriggerConfig {
    source: ServerAddress,
    source_expr: Expr,
    role: Option<Symbol>,
    codec: Symbol,
    decode_expr: Expr,
    decoder: TriggerDecoder,
    cron: Option<CronMatcher>,
    network_source: Option<Box<dyn TriggerSource>>,
}

#[derive(Default)]
struct TriggerState {
    file_offset: usize,
    file_remainder: Vec<u8>,
    delivered: u64,
    source_closed: bool,
    last_cron_minute: Option<u64>,
}

enum StdinSource {
    Channel(Mutex<Receiver<Option<Vec<u8>>>>),
    Unavailable,
}

#[non_citizen(
    reason = "live trigger handle; reconstruct source through server/Address descriptor and trigger ops",
    kind = "handle"
)]
/// Live handle to a running trigger that delivers source events into a server.
pub struct TriggerHandle {
    id: u64,
    server: Arc<Server>,
    source: ServerAddress,
    source_expr: Expr,
    role: Option<Symbol>,
    codec: Symbol,
    decode_expr: Expr,
    decoder: TriggerDecoder,
    cron: Option<CronMatcher>,
    network_source: Option<Mutex<Box<dyn TriggerSource>>>,
    stopping: AtomicBool,
    handle: Mutex<Option<JoinHandle<()>>>,
    stdin: StdinSource,
    state: Mutex<TriggerState>,
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn enqueue_trigger_event(source: &ServerAddress, payload: Vec<u8>) -> Result<()> {
    let key = match source {
        ServerAddress::Webhook { route } => Some(format!("webhook:{route}")),
        ServerAddress::Imap { address, mailbox } => Some(format!("imap:{address}:{mailbox}")),
        ServerAddress::Smtp { address } => Some(format!("smtp:{address}")),
        ServerAddress::Telegram { chat_id, bot } => Some(format!("telegram:{chat_id}:{bot}")),
        ServerAddress::Matrix { room_id } => Some(format!("matrix:{room_id}")),
        _ => None,
    }
    .ok_or_else(|| Error::Eval("source does not use the queued trigger runtime".to_owned()))?;
    let mut queues = queued_trigger_events()
        .lock()
        .map_err(|_| Error::PoisonedLock("trigger queue"))?;
    queues.entry(key).or_default().push(payload);
    Ok(())
}

#[cfg(test)]
pub(crate) fn loopback_smtp_messages_for(source: &ServerAddress) -> Result<Vec<Vec<u8>>> {
    let ServerAddress::Smtp { address } = source else {
        return Err(Error::Eval(
            "source does not use the loopback smtp runtime".to_owned(),
        ));
    };
    let key = format!("smtp:{address}");
    let messages = loopback_smtp_messages()
        .lock()
        .map_err(|_| Error::PoisonedLock("loopback smtp messages"))?;
    Ok(messages.get(&key).cloned().unwrap_or_default())
}

impl Object for TriggerHandle {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<server-trigger>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for TriggerHandle {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "Trigger"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.reflect_value(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        self.reflect_value(cx)
    }
}

pub(crate) fn register_trigger(
    cx: &mut Cx,
    server: Arc<Server>,
    source_expr: Expr,
    decode_expr: Expr,
    role: Option<Symbol>,
    codec: Symbol,
) -> Result<Arc<TriggerHandle>> {
    let source_expr = literal_expr(&source_expr).clone();
    let source = ServerAddress::from_expr(&source_expr)?;
    if let Some(capability) = source_capability(&source) {
        cx.require(&capability)?;
    }
    let cron = match &source {
        ServerAddress::Cron { spec } => Some(CronMatcher::parse(spec)?),
        _ => None,
    };
    let network_source = build_network_source(&source, &source_expr)?;

    ensure_installed_codec(cx, &codec)?;
    let decoder = build_decoder(cx, &decode_expr)?;
    let trigger = Arc::new(TriggerHandle::new(
        server.clone(),
        TriggerConfig {
            source,
            source_expr,
            role,
            codec,
            decode_expr,
            decoder,
            cron,
            network_source,
        },
    ));
    server.register_trigger(trigger.clone())?;
    trigger.start(cx)?;
    Ok(trigger)
}

fn build_decoder(cx: &mut Cx, decode_expr: &Expr) -> Result<TriggerDecoder> {
    let expr = literal_expr(decode_expr).clone();
    if let Expr::Symbol(symbol) = &expr
        && let Some(codec) = normalize_codec_symbol(cx, symbol)
    {
        ensure_installed_codec(cx, &codec)?;
        return Ok(TriggerDecoder::Codec(codec));
    }
    let callable = cx.eval_expr(expr)?;
    if callable.object().as_callable().is_none() {
        return Err(Error::TypeMismatch {
            expected: "callable or codec symbol",
            found: "non-callable",
        });
    }
    Ok(TriggerDecoder::Callable(callable))
}

fn normalize_codec_symbol(cx: &Cx, symbol: &Symbol) -> Option<Symbol> {
    if cx.registry().codec_by_symbol(symbol).is_some() {
        return Some(symbol.clone());
    }
    let qualified = Symbol::qualified("codec", symbol.name.to_string());
    if cx.registry().codec_by_symbol(&qualified).is_some() {
        return Some(qualified);
    }
    None
}

fn literal_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Quote { expr, .. } => expr,
        _ => expr,
    }
}

fn trigger_read_policy(cx: &Cx) -> ReadPolicy {
    let mut capabilities = sim_kernel::CapabilitySet::new();
    if cx.require(&read_construct_capability()).is_ok() {
        capabilities.insert(read_construct_capability());
    }
    if cx.require(&read_eval_capability()).is_ok() {
        capabilities.insert(read_eval_capability());
    }
    ReadPolicy {
        trust: TrustLevel::TrustedSource,
        capabilities,
    }
}

fn io_error_to_host(err: std::io::Error) -> Error {
    Error::host_io(err)
}

fn delivery_timeout() -> Duration {
    Duration::from_millis(250)
}

fn source_loopback_enabled(expr: &Expr) -> bool {
    source_loopback(expr).unwrap_or(false)
}
