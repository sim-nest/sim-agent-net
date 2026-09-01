#[derive(Clone)]
pub(crate) struct AgentRuntimeSite {
    capabilities: Vec<CapabilityName>,
    recorders: Vec<Value>,
    model_runners: Vec<Value>,
    model_tools: Vec<Value>,
    conduct: Option<Value>,
    budget: Option<usize>,
    result_shape: Option<Value>,
    inner: Arc<dyn EvalSite>,
}

#[derive(Clone)]
pub(crate) struct AgentRuntimeConfig {
    capabilities: Vec<CapabilityName>,
    recorders: Vec<Value>,
    model_runners: Vec<Value>,
    model_tools: Vec<Value>,
    conduct: Option<Value>,
    budget: Option<usize>,
    result_shape: Option<Value>,
}

#[derive(Clone)]
struct IdentityEvalSite {
    codecs: Vec<Symbol>,
}

#[derive(Clone)]
struct RouterTapSite {
    codecs: Vec<Symbol>,
    inner: Arc<dyn EvalSite>,
}

#[derive(Clone)]
struct RecorderSnifferSite {
    inner: Arc<dyn EvalSite>,
    recorders: Vec<Value>,
    stage: String,
    tool: Option<Symbol>,
}

impl AgentRuntimeSite {
    pub(crate) fn new(config: AgentRuntimeConfig, inner: Arc<dyn EvalSite>) -> Self {
        Self {
            capabilities: config.capabilities,
            recorders: config.recorders,
            model_runners: config.model_runners,
            model_tools: config.model_tools,
            conduct: config.conduct,
            budget: config.budget,
            result_shape: config.result_shape,
            inner,
        }
    }
}

impl EvalSite for AgentRuntimeSite {
    fn site_kind(&self) -> &'static str {
        "agent"
    }

    fn address(&self) -> &ServerAddress {
        self.inner.address()
    }

    fn codecs(&self) -> &[Symbol] {
        self.inner.codecs()
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let mut frame = frame;
        for capability in &self.capabilities {
            if frame
                .envelope
                .required_capabilities
                .iter()
                .any(|required| required == capability)
            {
                cx.require(capability)?;
            }
        }
        if frame.kind != FrameKind::Request {
            return Err(Error::Eval(
                "agent runtime only answers request frames".to_owned(),
            ));
        }
        frame = narrow_run_frame(cx, frame, self.budget, self.result_shape.as_ref())?;
        let task_id = ensure_task_id(&mut frame);
        with_task_id(task_id, || {
            record_trace_entry(cx, &self.recorders, &frame, "agent", "before", None)?;
            let reply = if self.conduct.is_some() {
                self.inner.answer(cx, frame)?
            } else if self.should_route_model_request(cx, &frame)? {
                self.answer_model_request(cx, frame)?
            } else {
                self.inner.answer(cx, frame)?
            };
            record_trace_entry(cx, &self.recorders, &reply, "agent", "after", None)?;
            Ok(reply)
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl AgentRuntimeSite {
    fn should_route_model_request(&self, cx: &mut Cx, frame: &ServerFrame) -> Result<bool> {
        if self.model_runners.is_empty() {
            return Ok(false);
        }
        let request = eval_request_from_frame(cx, frame)?;
        Ok(is_model_request(&request.expr))
    }

    fn answer_model_request(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let consistency = frame.envelope.consistency;
        let reply_codec = frame.codec.clone();
        let msg_id = frame.msg_id;
        let correlate = frame.correlate;
        let mut request = eval_request_from_frame(cx, &frame)?;
        request.expr = inject_manifest_tools(cx, request.expr, &self.model_tools)?;
        let runner = select_runner(cx, &self.model_runners, frame.envelope.role.as_ref())?;
        let runner_site = crate::agents::site_from_value(runner)?;
        let mut runner_frame =
            server_frame_from_request(cx, &first_codec(runner_site.codecs()), request)?;
        runner_frame.msg_id = msg_id;
        runner_frame.correlate = correlate.or(msg_id);
        runner_frame.envelope.role = Some(Symbol::new("runner"));
        record_trace_entry(
            cx,
            &self.recorders,
            &runner_frame,
            "agent-tool-injection",
            "after",
            None,
        )?;
        record_trace_entry(cx, &self.recorders, &runner_frame, "runner", "before", None)?;
        let mut runner_reply = runner_site.answer(cx, runner_frame.clone())?;
        if runner_reply.msg_id.is_none() {
            runner_reply.msg_id = runner_frame.msg_id;
        }
        if runner_reply.correlate.is_none() {
            runner_reply.correlate = runner_frame.msg_id;
        }
        record_trace_entry(cx, &self.recorders, &runner_reply, "runner", "after", None)?;
        let reply = eval_reply_from_frame(cx, &runner_reply)?;
        server_frame_from_reply(cx, &reply_codec, reply, consistency)
    }
}

impl EvalSite for IdentityEvalSite {
    fn site_kind(&self) -> &'static str {
        "identity"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if frame.kind != FrameKind::Request {
            return Err(Error::Eval(
                "identity eval site only answers request frames".to_owned(),
            ));
        }
        let consistency = frame.envelope.consistency;
        let reply_codec = frame.codec.clone();
        let request = eval_request_from_frame(cx, &frame)?;
        let value = crate::value_from_expr(cx, &request.expr)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &reply_codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for RouterTapSite {
    fn site_kind(&self) -> &'static str {
        "router-tap"
    }

    fn address(&self) -> &ServerAddress {
        self.inner.address()
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if frame.kind != FrameKind::Request {
            return self.inner.answer(cx, frame);
        }
        let consistency = frame.envelope.consistency;
        let reply_codec = frame.codec.clone();
        let request = eval_request_from_frame(cx, &frame)?;
        let _ = self.inner.answer(cx, frame)?;
        let value = crate::value_from_expr(cx, &request.expr)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &reply_codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for RecorderSnifferSite {
    fn site_kind(&self) -> &'static str {
        "recorder-sniffer"
    }

    fn address(&self) -> &ServerAddress {
        self.inner.address()
    }

    fn codecs(&self) -> &[Symbol] {
        self.inner.codecs()
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let mut frame = frame;
        ensure_task_id(&mut frame);
        record_trace_entry(
            cx,
            &self.recorders,
            &frame,
            &self.stage,
            "before",
            self.tool.as_ref(),
        )?;
        let mut reply = self.inner.answer(cx, frame.clone())?;
        if reply.msg_id.is_none() {
            reply.msg_id = frame.msg_id;
        }
        if reply.correlate.is_none() {
            reply.correlate = frame.msg_id;
        }
        record_trace_entry(
            cx,
            &self.recorders,
            &reply,
            &self.stage,
            "after",
            self.tool.as_ref(),
        )?;
        Ok(reply)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
