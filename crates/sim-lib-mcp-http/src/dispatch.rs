/// Boundary implemented by the modern service and explicit legacy adapters.
pub trait McpDispatch: Send + Sync {
    /// Dispatches one checked envelope with request-owned context and cancellation.
    fn dispatch(
        &self,
        context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>>;
}

/// Concrete dispatcher for the immutable modern [`McpService`].
pub struct ServiceDispatch {
    service: McpService,
    host_seed: Mutex<Cx>,
}

impl ServiceDispatch {
    /// Binds the service to an explicitly prepared host context seed.
    pub fn new(service: McpService, host_seed: Cx) -> Self {
        Self {
            service,
            host_seed: Mutex::new(host_seed),
        }
    }
}

impl McpDispatch for ServiceDispatch {
    fn dispatch(
        &self,
        context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        if cancellation.is_cancelled() {
            return Err(Error::Eval("MCP HTTP request cancelled".into()));
        }
        match envelope {
            McpEnvelope::Request(request) => {
                let mut host_seed = self
                    .host_seed
                    .lock()
                    .map_err(|_| Error::PoisonedLock("MCP service seed"))?;
                self.service
                    .handle(&mut host_seed, context, request)
                    .map(Iterator::collect)
            }
            McpEnvelope::Notification(_) => Ok(Vec::new()),
            _ => Err(Error::Eval(
                "MCP HTTP dispatch accepts only requests and notifications".into(),
            )),
        }
    }
}

/// Explicit initialize-era dispatcher for a distinct legacy endpoint.
pub struct LegacyDispatch {
    connection: Mutex<(LegacyConnection, Cx)>,
}
impl LegacyDispatch {
    /// Binds one intentionally stateful compatibility connection and its host context.
    pub fn new(connection: LegacyConnection, host_seed: Cx) -> Self {
        Self {
            connection: Mutex::new((connection, host_seed)),
        }
    }
}
impl McpDispatch for LegacyDispatch {
    fn dispatch(
        &self,
        _context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        if cancellation.is_cancelled() {
            return Err(Error::Eval("legacy MCP HTTP request cancelled".into()));
        }
        let mut state = self
            .connection
            .lock()
            .map_err(|_| Error::PoisonedLock("legacy MCP connection"))?;
        let (connection, cx) = &mut *state;
        connection.handle_envelope(cx, envelope)
    }
}
