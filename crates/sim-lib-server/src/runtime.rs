use std::thread::JoinHandle;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use sim_kernel::{Cx, Error, Result};

use crate::{ConnectionTransport, IsolationPolicy, ServerTransport, Session, ThreadMode};

/// Server-side runtime: owns the transport, tracks live sessions and worker threads, and
/// records connection and message counters.
pub struct ServerRuntime {
    transport: Arc<dyn ServerTransport>,
    thread_mode: ThreadMode,
    sessions: Mutex<BTreeMap<u64, Session>>,
    cx: Mutex<Cx>,
    accept_thread: Mutex<Option<JoinHandle<()>>>,
    worker_threads: Mutex<Vec<JoinHandle<()>>>,
    next_session_id: AtomicU64,
    connections: AtomicU64,
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    max_inflight: usize,
    session_isolation: IsolationPolicy,
    stopping: AtomicBool,
}

impl ServerRuntime {
    /// Creates a runtime with the default session isolation policy.
    pub fn new(
        transport: Arc<dyn ServerTransport>,
        cx: Cx,
        thread_mode: ThreadMode,
        max_inflight: usize,
    ) -> Self {
        Self::new_with_isolation(
            transport,
            cx,
            thread_mode,
            max_inflight,
            IsolationPolicy::default(),
        )
    }

    /// Creates a runtime with an explicit session isolation policy applied to new sessions.
    pub fn new_with_isolation(
        transport: Arc<dyn ServerTransport>,
        cx: Cx,
        thread_mode: ThreadMode,
        max_inflight: usize,
        session_isolation: IsolationPolicy,
    ) -> Self {
        Self {
            transport,
            thread_mode,
            sessions: Mutex::new(BTreeMap::new()),
            cx: Mutex::new(cx),
            accept_thread: Mutex::new(None),
            worker_threads: Mutex::new(Vec::new()),
            next_session_id: AtomicU64::new(1),
            connections: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            max_inflight,
            session_isolation,
            stopping: AtomicBool::new(false),
        }
    }

    /// Returns the transport this runtime accepts connections on.
    pub fn transport(&self) -> &Arc<dyn ServerTransport> {
        &self.transport
    }

    /// Returns the threading mode used to service connections.
    pub fn thread_mode(&self) -> &ThreadMode {
        &self.thread_mode
    }

    /// Returns the number of currently open sessions.
    pub fn session_count(&self) -> usize {
        self.sessions
            .lock()
            .map(|sessions| sessions.len())
            .unwrap_or(0)
    }

    /// Returns a snapshot of all open sessions.
    pub fn sessions(&self) -> Result<Vec<Session>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| Error::HostError("server runtime session mutex poisoned".to_owned()))?;
        Ok(sessions.values().cloned().collect())
    }

    /// Signals that the runtime should stop accepting and servicing connections.
    pub fn begin_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Returns whether a stop has been requested.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Opens a new session with the given codec and isolation policy, returning its id.
    pub fn open_session(
        &self,
        negotiated_codec: sim_kernel::Symbol,
        isolation: crate::IsolationPolicy,
    ) -> Result<u64> {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        self.connections.fetch_add(1, Ordering::Relaxed);
        self.sessions
            .lock()
            .map_err(|_| Error::HostError("server runtime session mutex poisoned".to_owned()))?
            .insert(
                session_id,
                Session {
                    id: session_id,
                    negotiated_codec,
                    isolation,
                    closed: false,
                },
            );
        Ok(session_id)
    }

    /// Updates the negotiated codec for an existing session; a no-op if the session is gone.
    pub fn update_session_codec(
        &self,
        session_id: u64,
        negotiated_codec: sim_kernel::Symbol,
    ) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| Error::HostError("server runtime session mutex poisoned".to_owned()))?;
        let Some(session) = sessions.get_mut(&session_id) else {
            return Ok(());
        };
        session.negotiated_codec = negotiated_codec;
        Ok(())
    }

    /// Removes the session with `session_id` from the runtime.
    pub fn close_session(&self, session_id: u64) -> Result<()> {
        self.sessions
            .lock()
            .map_err(|_| Error::HostError("server runtime session mutex poisoned".to_owned()))?
            .remove(&session_id);
        Ok(())
    }

    /// Removes all sessions from the runtime.
    pub fn clear_sessions(&self) -> Result<()> {
        self.sessions
            .lock()
            .map_err(|_| Error::HostError("server runtime session mutex poisoned".to_owned()))?
            .clear();
        Ok(())
    }

    /// Returns the total number of connections opened over the runtime's lifetime.
    pub fn connection_count(&self) -> u64 {
        self.connections.load(Ordering::Relaxed)
    }

    /// Returns the total number of messages sent.
    pub fn messages_sent(&self) -> u64 {
        self.messages_sent.load(Ordering::Relaxed)
    }

    /// Returns the total number of messages received.
    pub fn messages_received(&self) -> u64 {
        self.messages_received.load(Ordering::Relaxed)
    }

    /// Returns the maximum number of in-flight requests permitted.
    pub fn max_inflight(&self) -> usize {
        self.max_inflight
    }

    /// Returns the isolation policy applied to new sessions.
    pub fn session_isolation(&self) -> &IsolationPolicy {
        &self.session_isolation
    }

    /// Increments the sent-message counter.
    pub fn note_message_sent(&self) {
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the received-message counter.
    pub fn note_message_received(&self) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    /// Runs `f` with exclusive access to the runtime's shared evaluation context.
    pub fn with_cx<T>(&self, f: impl FnOnce(&mut Cx) -> Result<T>) -> Result<T> {
        let mut cx = self
            .cx
            .lock()
            .map_err(|_| Error::HostError("server runtime cx mutex poisoned".to_owned()))?;
        f(&mut cx)
    }

    /// Stores the join handle for the connection-accept thread.
    pub fn set_accept_thread(&self, handle: JoinHandle<()>) -> Result<()> {
        let mut slot = self.accept_thread.lock().map_err(|_| {
            Error::HostError("server runtime accept-thread mutex poisoned".to_owned())
        })?;
        *slot = Some(handle);
        Ok(())
    }

    /// Joins the accept thread if one is registered.
    pub fn join_accept_thread(&self) -> Result<()> {
        let handle = self
            .accept_thread
            .lock()
            .map_err(|_| {
                Error::HostError("server runtime accept-thread mutex poisoned".to_owned())
            })?
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        Ok(())
    }

    /// Records a worker thread's join handle for later cleanup.
    pub fn register_worker_thread(&self, handle: JoinHandle<()>) -> Result<()> {
        self.worker_threads
            .lock()
            .map_err(|_| {
                Error::HostError("server runtime worker-thread mutex poisoned".to_owned())
            })?
            .push(handle);
        Ok(())
    }

    /// Joins and drains all registered worker threads.
    pub fn join_worker_threads(&self) -> Result<()> {
        let handles = std::mem::take(&mut *self.worker_threads.lock().map_err(|_| {
            Error::HostError("server runtime worker-thread mutex poisoned".to_owned())
        })?);
        for handle in handles {
            let _ = handle.join();
        }
        Ok(())
    }

    /// Waits up to `timeout` for an incoming connection, returning its transport if one arrives.
    pub fn accept_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        self.with_cx(|cx| self.transport.accept_timeout(cx, timeout))
    }
}
