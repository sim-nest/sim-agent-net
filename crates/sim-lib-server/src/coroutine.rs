use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

use sim_citizen_derive::non_citizen;
use sim_kernel::{Args, ClassRef, Cx, Error, Object, Result, Symbol, Value};

use crate::ServerAddress;

static NEXT_COROUTINE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Lifecycle state of a [`Coroutine`].
pub enum CoroutineStatus {
    /// The coroutine handler is currently executing.
    Running,
    /// The coroutine has yielded and is waiting to be resumed.
    Suspended,
    /// The coroutine has finished, errored, or been cancelled.
    Done,
}

impl CoroutineStatus {
    fn as_u8(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Suspended => 1,
            Self::Done => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Running,
            1 => Self::Suspended,
            2 => Self::Done,
            _ => Self::Done,
        }
    }

    /// Returns the status as its `running`/`suspended`/`done` symbol.
    pub fn as_symbol(self) -> Symbol {
        Symbol::new(match self {
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Done => "done",
        })
    }
}

#[derive(Default)]
struct CoroutineState {
    mailbox: Option<Value>,
    yielded: Option<Value>,
    cancelled: bool,
}

struct CoroutineInner {
    id: u64,
    address: ServerAddress,
    handler: Value,
    status: AtomicU8,
    state: Mutex<CoroutineState>,
}

#[derive(Clone)]
#[non_citizen(
    reason = "live coroutine handle; inspect through server/Frame descriptor and coroutine ops",
    kind = "handle"
)]
/// Live, resumable coroutine bound to a server address and handler value.
///
/// Cloning shares the same underlying state across handles.
pub struct Coroutine {
    inner: Arc<CoroutineInner>,
}

impl Coroutine {
    /// Creates a suspended coroutine for `address` driven by `handler`.
    pub fn new(address: ServerAddress, handler: Value) -> Self {
        Self {
            inner: Arc::new(CoroutineInner {
                id: NEXT_COROUTINE_ID.fetch_add(1, Ordering::Relaxed),
                address,
                handler,
                status: AtomicU8::new(CoroutineStatus::Suspended.as_u8()),
                state: Mutex::new(CoroutineState::default()),
            }),
        }
    }

    /// Returns the unique, process-local coroutine id.
    pub fn id(&self) -> u64 {
        self.inner.id
    }

    /// Returns the server address this coroutine is bound to.
    pub fn address(&self) -> &ServerAddress {
        &self.inner.address
    }

    /// Returns the current lifecycle status.
    pub fn status(&self) -> CoroutineStatus {
        CoroutineStatus::from_u8(self.inner.status.load(Ordering::Relaxed))
    }

    /// Resumes the coroutine, delivering `input` and running until it yields or
    /// finishes.
    ///
    /// Returns the yielded value if the handler yielded, otherwise the
    /// handler's final result. Errors if the coroutine is already done or has
    /// been cancelled.
    pub fn resume(&self, cx: &mut Cx, input: Value) -> Result<Value> {
        if self.status() == CoroutineStatus::Done {
            return Err(Error::Eval("coroutine is done".to_owned()));
        }

        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| Error::PoisonedLock("coroutine"))?;
            if state.cancelled {
                self.inner
                    .status
                    .store(CoroutineStatus::Done.as_u8(), Ordering::Relaxed);
                return Err(Error::Eval("coroutine is cancelled".to_owned()));
            }
            state.mailbox = Some(input.clone());
            state.yielded = None;
        }

        self.inner
            .status
            .store(CoroutineStatus::Running.as_u8(), Ordering::Relaxed);

        let handle = cx.factory().opaque(Arc::new(self.clone()))?;
        let result = cx.call_value(self.inner.handler.clone(), Args::new(vec![handle, input]));

        let yielded = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| Error::PoisonedLock("coroutine"))?;
            state.mailbox = None;
            state.yielded.take()
        };

        match result {
            Ok(value) => {
                if let Some(yielded) = yielded {
                    self.inner
                        .status
                        .store(CoroutineStatus::Suspended.as_u8(), Ordering::Relaxed);
                    Ok(yielded)
                } else {
                    self.inner
                        .status
                        .store(CoroutineStatus::Done.as_u8(), Ordering::Relaxed);
                    Ok(value)
                }
            }
            Err(err) => {
                self.inner
                    .status
                    .store(CoroutineStatus::Done.as_u8(), Ordering::Relaxed);
                Err(err)
            }
        }
    }

    /// Records `value` as the coroutine's yield and returns it.
    ///
    /// Only valid from a running coroutine; errors otherwise.
    pub fn yield_value(&self, value: Value) -> Result<Value> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("coroutine"))?;
        if self.status() != CoroutineStatus::Running {
            return Err(Error::Eval(
                "server/yield may only be used from a running coroutine".to_owned(),
            ));
        }
        state.yielded = Some(value.clone());
        Ok(value)
    }

    /// Cancels the coroutine, clearing its state and marking it done.
    pub fn cancel(&self) -> Result<()> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("coroutine"))?;
        state.cancelled = true;
        state.mailbox = None;
        state.yielded = None;
        self.inner
            .status
            .store(CoroutineStatus::Done.as_u8(), Ordering::Relaxed);
        Ok(())
    }
}

impl Object for Coroutine {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<server-coroutine>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for Coroutine {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "Coroutine"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<sim_kernel::Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("coroutine"))?;
        let mailbox = match &state.mailbox {
            Some(mailbox) => mailbox.clone(),
            None => cx.factory().nil()?,
        };
        let cancelled = state.cancelled;
        drop(state);
        let address = self.inner.address.as_value(cx)?;
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("coroutine"))?,
            ),
            (
                Symbol::new("id"),
                cx.factory().string(self.inner.id.to_string())?,
            ),
            (
                Symbol::new("status"),
                cx.factory().symbol(self.status().as_symbol())?,
            ),
            (Symbol::new("address"), address),
            (Symbol::new("mailbox"), mailbox),
            (Symbol::new("cancelled"), cx.factory().bool(cancelled)?),
        ])
    }
}
