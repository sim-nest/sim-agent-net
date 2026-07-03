use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use sim_citizen_derive::non_citizen;
use sim_kernel::{
    CapabilityName, ClassRef, Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest,
    Expr, Object, Result, Symbol, Value, eval_remote_capability,
};

use crate::{
    EvalSite, FrameKind, FrameRouter, IsolationPolicy, ServerAddress, ServerFrame,
    eval_reply_from_frame, server_frame_from_request, symbol_list_value,
};

/// State of a single server connection session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Session id, unique within the owning runtime.
    pub id: u64,
    /// Codec negotiated for this session's frames.
    pub negotiated_codec: Symbol,
    /// Isolation policy applied to evaluation in this session.
    pub isolation: IsolationPolicy,
    /// Whether the session has been closed.
    pub closed: bool,
}

impl Session {
    pub(crate) fn as_value(&self, cx: &mut Cx) -> Result<Value> {
        let isolation = self.isolation.as_value(cx)?;
        cx.factory().table(vec![
            (Symbol::new("id"), cx.factory().string(self.id.to_string())?),
            (
                Symbol::new("negotiated-codec"),
                cx.factory().symbol(self.negotiated_codec.clone())?,
            ),
            (Symbol::new("isolation"), isolation),
            (Symbol::new("closed"), cx.factory().bool(self.closed)?),
        ])
    }
}

#[derive(Clone)]
#[non_citizen(
    reason = "live server connection handle; reconstruct via server/Address descriptor and connect ops",
    kind = "handle"
)]
/// Live client handle to a server endpoint, evaluating expressions over a transport [`EvalSite`].
pub struct Connection {
    address: ServerAddress,
    default_codec: Symbol,
    supported_codecs: Vec<Symbol>,
    site: Arc<dyn EvalSite>,
    session: Arc<Mutex<Session>>,
    router: Arc<FrameRouter>,
    role: Option<Symbol>,
}

impl Connection {
    /// Opens a connection with default role and isolation policy.
    pub fn new(
        address: ServerAddress,
        default_codec: Symbol,
        supported_codecs: Vec<Symbol>,
        site: Arc<dyn EvalSite>,
    ) -> Result<Self> {
        Self::with_session(
            address,
            default_codec,
            supported_codecs,
            site,
            None,
            IsolationPolicy::default(),
        )
    }

    /// Opens a connection with an explicit `role` and `isolation` policy, verifying the
    /// address transport is available before constructing the initial session.
    pub fn with_session(
        address: ServerAddress,
        default_codec: Symbol,
        supported_codecs: Vec<Symbol>,
        site: Arc<dyn EvalSite>,
        role: Option<Symbol>,
        isolation: IsolationPolicy,
    ) -> Result<Self> {
        address.ensure_transport_available()?;
        Ok(Self {
            address,
            default_codec: default_codec.clone(),
            supported_codecs,
            site,
            session: Arc::new(Mutex::new(Session {
                id: 0,
                negotiated_codec: default_codec,
                isolation,
                closed: false,
            })),
            router: Arc::new(FrameRouter::default()),
            role,
        })
    }

    /// Returns the remote address this connection targets.
    pub fn address(&self) -> &ServerAddress {
        &self.address
    }

    /// Returns the codec used by default for outbound frames.
    pub fn default_codec(&self) -> &Symbol {
        &self.default_codec
    }

    /// Returns the codecs this connection is willing to negotiate.
    pub fn supported_codecs(&self) -> &[Symbol] {
        &self.supported_codecs
    }

    /// Returns the underlying eval site that carries frames.
    pub fn site(&self) -> &Arc<dyn EvalSite> {
        &self.site
    }

    /// Returns a snapshot of the current session, or a closed placeholder if the lock is poisoned.
    pub fn session(&self) -> Session {
        self.session
            .lock()
            .map(|session| session.clone())
            .unwrap_or(Session {
                id: 0,
                negotiated_codec: self.default_codec.clone(),
                isolation: IsolationPolicy::default(),
                closed: true,
            })
    }

    /// Returns the role symbol attached to outbound frames, if any.
    pub fn role(&self) -> Option<&Symbol> {
        self.role.as_ref()
    }

    /// Evaluates `expr` remotely and returns its result value, using the connection's
    /// default consistency.
    pub fn request(
        &self,
        cx: &mut Cx,
        expr: Expr,
        timeout: Option<Duration>,
        required_capabilities: Vec<CapabilityName>,
    ) -> Result<Value> {
        self.request_with_consistency(
            cx,
            expr,
            timeout,
            required_capabilities,
            self.default_consistency(),
        )
    }

    pub(crate) fn request_with_consistency(
        &self,
        cx: &mut Cx,
        expr: Expr,
        timeout: Option<Duration>,
        required_capabilities: Vec<CapabilityName>,
        consistency: Consistency,
    ) -> Result<Value> {
        self.enforce_consistency(consistency)?;
        let request = EvalRequest {
            expr,
            result_shape: None,
            required_capabilities,
            deadline: timeout,
            consistency,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        };
        let reply = self.realize(cx, request)?;
        Ok(reply.value)
    }

    /// Sends `expr` as a request frame without awaiting a synchronous result, returning the
    /// assigned message id for later correlation. Uses the connection's default consistency.
    pub fn send(
        &self,
        cx: &mut Cx,
        expr: Expr,
        codec: Symbol,
        deadline: Option<Duration>,
        required_capabilities: Vec<CapabilityName>,
        reply_codec_hint: Option<Symbol>,
    ) -> Result<u64> {
        self.send_with_consistency(
            cx,
            expr,
            codec,
            deadline,
            required_capabilities,
            reply_codec_hint,
            self.default_consistency(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn send_with_consistency(
        &self,
        cx: &mut Cx,
        expr: Expr,
        codec: Symbol,
        deadline: Option<Duration>,
        required_capabilities: Vec<CapabilityName>,
        reply_codec_hint: Option<Symbol>,
        consistency: Consistency,
    ) -> Result<u64> {
        self.enforce_consistency(consistency)?;
        self.enforce_remote_capability(cx)?;
        let msg_id = self.router.fresh_msg_id();
        let mut frame = server_frame_from_request(
            cx,
            &codec,
            EvalRequest {
                expr,
                result_shape: None,
                required_capabilities,
                deadline,
                consistency,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )?;
        frame.kind = FrameKind::Request;
        frame.envelope.reply_codec_hint = reply_codec_hint;
        frame.envelope.role = self.role.clone();
        frame.msg_id = Some(msg_id);

        let mut reply = self.site.answer(cx, frame)?;
        if reply.correlate.is_none() {
            reply.correlate = Some(msg_id);
        }
        self.router.push_inbound(reply)?;
        Ok(msg_id)
    }

    /// Sends `expr` as a one-way notify frame, expecting no correlated reply.
    pub fn notify(
        &self,
        cx: &mut Cx,
        expr: Expr,
        codec: Symbol,
        deadline: Option<Duration>,
        required_capabilities: Vec<CapabilityName>,
    ) -> Result<()> {
        self.enforce_remote_capability(cx)?;
        let mut frame = ServerFrame::from_expr(
            cx,
            codec,
            FrameKind::Notify,
            &expr,
            self.default_consistency(),
            required_capabilities,
            false,
        )?;
        frame.envelope.deadline = deadline;
        frame.envelope.role = self.role.clone();
        let _ = self.site.answer(cx, frame)?;
        Ok(())
    }

    /// Pops the next buffered inbound frame, if one is queued.
    ///
    /// Returns [`Error::PoisonedLock`] when the router's queue mutex is
    /// poisoned, rather than panicking.
    pub fn receive(&self, _timeout: Option<Duration>) -> Result<Option<ServerFrame>> {
        self.router.pop_inbound()
    }

    /// Closes the connection and marks its session closed.
    pub fn close(&self, cx: &mut Cx) -> Result<()> {
        self.site.close_connection(cx)?;
        if let Ok(mut session) = self.session.lock() {
            session.closed = true;
        }
        Ok(())
    }
}

impl Object for Connection {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<server-connection>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for Connection {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "Connection"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let address = self.address.as_value(cx)?;
        let default_codec = cx.factory().symbol(self.default_codec.clone())?;
        let supported_codecs = symbol_list_value(cx, &self.supported_codecs)?;
        let site_kind = cx.factory().string(self.site.site_kind().to_owned())?;
        let site_address = self.site.address().as_value(cx)?;
        let site_codecs = symbol_list_value(cx, self.site.codecs())?;
        let session = self
            .session
            .lock()
            .map_err(|_| Error::HostError("connection session mutex poisoned".to_owned()))?
            .as_value(cx)?;
        let next_msg_id = cx
            .factory()
            .string(self.router.peek_next_msg_id().to_string())?;
        let role = match &self.role {
            Some(role) => cx.factory().symbol(role.clone())?,
            None => cx.factory().nil()?,
        };
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("connection"))?,
            ),
            (Symbol::new("address"), address),
            (Symbol::new("default-codec"), default_codec),
            (Symbol::new("supported-codecs"), supported_codecs),
            (Symbol::new("site-kind"), site_kind),
            (Symbol::new("site-address"), site_address),
            (Symbol::new("site-codecs"), site_codecs),
            (Symbol::new("session"), session),
            (Symbol::new("role"), role),
            (Symbol::new("next-msg-id"), next_msg_id),
        ])
    }
    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

impl EvalFabric for Connection {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.enforce_remote_capability(cx)?;
        let mut frame = server_frame_from_request(cx, self.default_codec(), request)?;
        frame.msg_id = Some(self.router.fresh_msg_id());
        frame.envelope.role = self.role.clone();
        let reply = self
            .site
            .answer_with_timeout(cx, frame.clone(), frame.envelope.deadline)?;
        eval_reply_from_frame(cx, &reply)
    }
}

impl Connection {
    fn requires_remote_capability(&self) -> bool {
        self.address.is_remote_like() || self.site.address().is_remote_like()
    }

    pub(crate) fn default_consistency(&self) -> Consistency {
        if self.requires_remote_capability() {
            Consistency::RemoteOnly
        } else {
            Consistency::LocalFirst
        }
    }

    fn enforce_remote_capability(&self, cx: &mut Cx) -> Result<()> {
        if self.requires_remote_capability() {
            cx.require(&eval_remote_capability())?;
        }
        Ok(())
    }

    pub(crate) fn enforce_consistency(&self, consistency: Consistency) -> Result<()> {
        match consistency {
            Consistency::LocalOnly if self.requires_remote_capability() => {
                Err(Error::Eval(format!(
                    "consistency local-only refuses remote address kind {}",
                    self.address.kind_symbol()
                )))
            }
            Consistency::RemoteOnly if !self.requires_remote_capability() => {
                Err(Error::Eval(format!(
                    "consistency remote-only refuses local address kind {}",
                    self.address.kind_symbol()
                )))
            }
            _ => Ok(()),
        }
    }
}
