use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};

use sim_citizen_derive::non_citizen;
use sim_kernel::{ClassRef, Cx, Expr, Object, Result, Symbol, Value};

use crate::{
    EvalSite, FrameRouter, IsolationPolicy, ServerAddress, ServerFrame, ServerRuntime,
    TriggerHandle, symbol_list_value,
};

static NEXT_SERVER_ID: AtomicU64 = AtomicU64::new(1);

/// Threading strategy a server uses to service connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadMode {
    /// Run on the calling main thread.
    Main,
    /// Run cooperatively, yielding to the host scheduler.
    Coop,
    /// Spawn a dedicated thread per connection.
    Spawn,
    /// Service connections from a shared worker pool.
    Pool,
    /// Run the wrapped mode as a coroutine.
    Coroutine(Box<ThreadMode>),
}

impl ThreadMode {
    /// Parses a thread mode from an expression: a bare symbol (`main`, `coop`, `spawn`,
    /// `pool`) or a `(coroutine <base>)` list.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::Symbol(symbol) => match symbol.name.as_ref() {
                "main" => Ok(Self::Main),
                "coop" => Ok(Self::Coop),
                "spawn" => Ok(Self::Spawn),
                "pool" => Ok(Self::Pool),
                other => Err(sim_kernel::Error::Eval(format!(
                    "unsupported thread mode {other}"
                ))),
            },
            Expr::List(items) | Expr::Vector(items) => {
                let Some(Expr::Symbol(head)) = items.first() else {
                    return Err(sim_kernel::Error::TypeMismatch {
                        expected: "thread mode list starting with a symbol",
                        found: "non-symbol",
                    });
                };
                if head.name.as_ref() != "coroutine" {
                    return Err(sim_kernel::Error::Eval(format!(
                        "unsupported thread mode {}",
                        head
                    )));
                }
                let base = match items.get(1) {
                    Some(expr) => Self::from_expr(expr)?,
                    None => Self::Coop,
                };
                Ok(Self::Coroutine(Box::new(base)))
            }
            _ => Err(sim_kernel::Error::TypeMismatch {
                expected: "thread mode expression",
                found: "non-thread-mode",
            }),
        }
    }

    /// Renders this thread mode back to its expression form.
    pub fn as_expr(&self) -> Expr {
        match self {
            Self::Main => Expr::Symbol(Symbol::new("main")),
            Self::Coop => Expr::Symbol(Symbol::new("coop")),
            Self::Spawn => Expr::Symbol(Symbol::new("spawn")),
            Self::Pool => Expr::Symbol(Symbol::new("pool")),
            Self::Coroutine(base) => {
                Expr::List(vec![Expr::Symbol(Symbol::new("coroutine")), base.as_expr()])
            }
        }
    }

    /// Returns whether this thread mode can be used in the current environment.
    pub fn is_available_now(&self) -> bool {
        match self {
            Self::Main | Self::Coop | Self::Spawn | Self::Pool => true,
            Self::Coroutine(base) => matches!(base.as_ref(), Self::Main | Self::Coop),
        }
    }

    /// Returns whether this mode is a coroutine variant.
    pub fn is_coroutine(&self) -> bool {
        matches!(self, Self::Coroutine(_))
    }
}

/// Lifecycle status of a running server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    /// Server is accepting and servicing connections.
    Running,
    /// Server is paused and not servicing connections.
    Suspended,
    /// Server has stopped.
    Stopped,
}

impl ServerStatus {
    fn as_u8(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Suspended => 1,
            Self::Stopped => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Running,
            1 => Self::Suspended,
            2 => Self::Stopped,
            _ => Self::Stopped,
        }
    }

    fn as_symbol(self) -> Symbol {
        Symbol::new(match self {
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Stopped => "stopped",
        })
    }
}

#[non_citizen(
    reason = "live server handle; reconstruct configuration via server/Address descriptor and start ops",
    kind = "handle",
    descriptor = "server/Address"
)]
/// Live server handle: an address bound to an [`EvalSite`], its codec and threading
/// configuration, lifecycle status, triggers, and optional [`ServerRuntime`].
pub struct Server {
    id: u64,
    name: Option<Symbol>,
    address: ServerAddress,
    default_codec: Symbol,
    supported_codecs: Vec<Symbol>,
    thread: ThreadMode,
    isolation: IsolationPolicy,
    status: AtomicU8,
    site: Arc<dyn EvalSite>,
    spec: Vec<(Symbol, Expr)>,
    router: Arc<FrameRouter>,
    triggers: Arc<Mutex<Vec<Arc<TriggerHandle>>>>,
    runtime: Option<Arc<ServerRuntime>>,
    started_at: Instant,
}

impl Server {
    /// Builds a server without an attached runtime.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: ServerAddress,
        default_codec: Symbol,
        supported_codecs: Vec<Symbol>,
        thread: ThreadMode,
        isolation: IsolationPolicy,
        name: Option<Symbol>,
        site: Arc<dyn EvalSite>,
        spec: Vec<(Symbol, Expr)>,
    ) -> Result<Self> {
        Self::with_runtime(
            address,
            default_codec,
            supported_codecs,
            thread,
            isolation,
            name,
            site,
            spec,
            None,
        )
    }

    /// Builds a server, optionally attaching a [`ServerRuntime`], after verifying the
    /// address transport is available.
    #[allow(clippy::too_many_arguments)]
    pub fn with_runtime(
        address: ServerAddress,
        default_codec: Symbol,
        supported_codecs: Vec<Symbol>,
        thread: ThreadMode,
        isolation: IsolationPolicy,
        name: Option<Symbol>,
        site: Arc<dyn EvalSite>,
        spec: Vec<(Symbol, Expr)>,
        runtime: Option<Arc<ServerRuntime>>,
    ) -> Result<Self> {
        address.ensure_transport_available()?;
        Ok(Self {
            id: NEXT_SERVER_ID.fetch_add(1, Ordering::Relaxed),
            name,
            address,
            default_codec,
            supported_codecs,
            thread,
            isolation,
            status: AtomicU8::new(ServerStatus::Running.as_u8()),
            site,
            spec,
            router: Arc::new(FrameRouter::default()),
            triggers: Arc::new(Mutex::new(Vec::new())),
            runtime,
            started_at: Instant::now(),
        })
    }

    /// Returns this server's unique id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the server's name, if one was assigned.
    pub fn name(&self) -> Option<&Symbol> {
        self.name.as_ref()
    }

    /// Returns the address the server is bound to.
    pub fn address(&self) -> &ServerAddress {
        &self.address
    }

    /// Returns the codec used by default for frames.
    pub fn default_codec(&self) -> &Symbol {
        &self.default_codec
    }

    /// Returns the codecs the server is willing to negotiate.
    pub fn supported_codecs(&self) -> &[Symbol] {
        &self.supported_codecs
    }

    /// Returns the server's threading mode.
    pub fn thread(&self) -> &ThreadMode {
        &self.thread
    }

    /// Returns the eval site that handles incoming frames.
    pub fn site(&self) -> &Arc<dyn EvalSite> {
        &self.site
    }

    /// Returns the isolation policy applied to sessions.
    pub fn isolation(&self) -> &IsolationPolicy {
        &self.isolation
    }

    /// Returns the configuration spec entries the server was started with.
    pub fn spec(&self) -> &[(Symbol, Expr)] {
        &self.spec
    }

    /// Returns the attached runtime, if the server is listening.
    pub fn runtime(&self) -> Option<&Arc<ServerRuntime>> {
        self.runtime.as_ref()
    }

    /// Returns the current lifecycle status.
    pub fn status(&self) -> ServerStatus {
        ServerStatus::from_u8(self.status.load(Ordering::Relaxed))
    }

    /// Sets the lifecycle status.
    pub fn set_status(&self, status: ServerStatus) {
        self.status.store(status.as_u8(), Ordering::Relaxed);
    }

    /// Returns the elapsed time since the server started, in milliseconds.
    pub fn uptime_millis(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Registers a trigger handle to be tracked and stopped with the server.
    pub fn register_trigger(&self, trigger: Arc<TriggerHandle>) -> Result<()> {
        self.triggers
            .lock()
            .map_err(|_| sim_kernel::Error::PoisonedLock("server triggers"))?
            .push(trigger);
        Ok(())
    }

    /// Stops every registered trigger.
    pub fn stop_triggers(&self) -> Result<()> {
        for trigger in self.trigger_snapshots()? {
            trigger.stop()?;
        }
        Ok(())
    }

    /// Buffers `frame` as inbound and delivers it to the eval site, as if fired by a trigger.
    pub fn deliver_trigger_frame(&self, cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        self.router.push_inbound(frame.clone())?;
        let _ = self.site.answer(cx, frame)?;
        Ok(())
    }

    /// Returns a snapshot of the currently registered trigger handles.
    pub fn trigger_snapshots(&self) -> Result<Vec<Arc<TriggerHandle>>> {
        Ok(self
            .triggers
            .lock()
            .map_err(|_| sim_kernel::Error::PoisonedLock("server triggers"))?
            .clone())
    }

    /// Returns a table value reflecting the server's configuration and live state.
    pub fn reflect_value(&self, cx: &mut Cx) -> Result<Value> {
        let mut entries = table_entries(self, cx)?;
        entries.extend(live_state_entries(self, cx)?);
        cx.factory().table(entries)
    }

    /// Returns a table value summarizing health: status, uptime, and session and message counts.
    pub fn health_value(&self, cx: &mut Cx) -> Result<Value> {
        let (sessions, connections, messages_sent, messages_received) = self
            .runtime
            .as_ref()
            .map(|runtime| {
                (
                    runtime.session_count(),
                    runtime.connection_count(),
                    runtime.messages_sent(),
                    runtime.messages_received(),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        cx.factory().table(vec![
            (
                Symbol::new("status"),
                cx.factory().symbol(self.status().as_symbol())?,
            ),
            (
                Symbol::new("uptime"),
                cx.factory().string(self.uptime_millis().to_string())?,
            ),
            (
                Symbol::new("sessions"),
                cx.factory().string(sessions.to_string())?,
            ),
            (
                Symbol::new("connections"),
                cx.factory().string(connections.to_string())?,
            ),
            (
                Symbol::new("messages-sent"),
                cx.factory().string(messages_sent.to_string())?,
            ),
            (
                Symbol::new("messages-received"),
                cx.factory().string(messages_received.to_string())?,
            ),
        ])
    }

    /// Returns a list value of the runtime's sessions, or an empty list if not listening.
    pub fn sessions_value(&self, cx: &mut Cx) -> Result<Value> {
        let Some(runtime) = &self.runtime else {
            return cx.factory().list(Vec::new());
        };
        let sessions = runtime
            .sessions()?
            .into_iter()
            .map(|session| session.as_value(cx))
            .collect::<Result<Vec<_>>>()?;
        cx.factory().list(sessions)
    }
}

impl Object for Server {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<server>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for Server {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "Server"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let mut entries = table_entries(self, cx)?;
        entries.extend(live_state_entries(self, cx)?);
        cx.factory().table(entries)
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            address: self.address.clone(),
            default_codec: self.default_codec.clone(),
            supported_codecs: self.supported_codecs.clone(),
            thread: self.thread.clone(),
            isolation: self.isolation.clone(),
            status: AtomicU8::new(self.status().as_u8()),
            site: self.site.clone(),
            spec: self.spec.clone(),
            router: self.router.clone(),
            triggers: self.triggers.clone(),
            runtime: self.runtime.clone(),
            started_at: self.started_at,
        }
    }
}

fn table_entries(server: &Server, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
    let name = match server.name() {
        Some(name) => cx.factory().symbol(name.clone())?,
        None => cx.factory().nil()?,
    };
    let spec_entries = server
        .spec()
        .iter()
        .map(|(key, value)| {
            cx.factory()
                .expr(Expr::List(vec![Expr::Symbol(key.clone()), value.clone()]))
        })
        .collect::<Result<Vec<_>>>()?;
    let spec = cx.factory().list(spec_entries)?;
    let address = server.address.as_value(cx)?;
    let default_codec = cx.factory().symbol(server.default_codec.clone())?;
    let supported_codecs = symbol_list_value(cx, &server.supported_codecs)?;
    let thread = cx.factory().expr(server.thread.as_expr())?;
    let site_kind = cx.factory().string(server.site.site_kind().to_owned())?;
    let site_address = server.site.address().as_value(cx)?;
    let site_codecs = symbol_list_value(cx, server.site.codecs())?;
    let isolation = server.isolation.as_value(cx)?;
    let listening = cx.factory().bool(server.runtime.is_some())?;
    let next_msg_id = cx
        .factory()
        .string(server.router.peek_next_msg_id().to_string())?;
    Ok(vec![
        (
            Symbol::new("kind"),
            cx.factory().symbol(Symbol::new("server"))?,
        ),
        (
            Symbol::new("id"),
            cx.factory().string(server.id.to_string())?,
        ),
        (Symbol::new("name"), name),
        (Symbol::new("address"), address),
        (Symbol::new("default-codec"), default_codec),
        (Symbol::new("supported-codecs"), supported_codecs),
        (Symbol::new("thread"), thread),
        (Symbol::new("site-kind"), site_kind),
        (Symbol::new("site-address"), site_address),
        (Symbol::new("site-codecs"), site_codecs),
        (Symbol::new("isolation"), isolation),
        (Symbol::new("listening"), listening),
        (Symbol::new("spec"), spec),
        (Symbol::new("next-msg-id"), next_msg_id),
    ])
}

fn live_state_entries(server: &Server, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
    let trigger_values = server
        .trigger_snapshots()?
        .into_iter()
        .map(|trigger| trigger.reflect_value(cx))
        .collect::<Result<Vec<_>>>()?;
    let triggers = cx.factory().list(trigger_values)?;
    let (sessions, connections, messages_sent, messages_received) = server
        .runtime
        .as_ref()
        .map(|runtime| {
            (
                runtime.session_count(),
                runtime.connection_count(),
                runtime.messages_sent(),
                runtime.messages_received(),
            )
        })
        .unwrap_or((0, 0, 0, 0));
    Ok(vec![
        (
            Symbol::new("status"),
            cx.factory().symbol(server.status().as_symbol())?,
        ),
        (
            Symbol::new("uptime"),
            cx.factory().string(server.uptime_millis().to_string())?,
        ),
        (
            Symbol::new("sessions"),
            cx.factory().string(sessions.to_string())?,
        ),
        (
            Symbol::new("connections"),
            cx.factory().string(connections.to_string())?,
        ),
        (
            Symbol::new("messages-sent"),
            cx.factory().string(messages_sent.to_string())?,
        ),
        (
            Symbol::new("messages-received"),
            cx.factory().string(messages_received.to_string())?,
        ),
        (Symbol::new("triggers"), triggers),
        (Symbol::new("line-driver"), cx.factory().nil()?),
    ])
}
