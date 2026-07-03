use super::super::tool_injection::{inject_manifest_tools, is_model_request};
use super::super::{collect_agent_components, first_codec};
use super::types::AgentState;
use super::{
    Agent, AgentFabric, AgentManifest, LoopbackStream, RuntimeValueSite, connection_from_value,
    site_from_value, swarm_status_value_for_table,
};
use crate::{AgentComponent, BlackboardMemory, FileMemory, Tool, WorkingMemory, expr_to_value};
use crate::{PersonaMemory, VectorMemory};
use sim_kernel::{ClassRef, Cx, Error, EvalReply, Expr, Object, Result, Symbol, Value};
use sim_lib_server::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, eval_reply_from_frame,
    eval_request_from_frame, server_frame_from_reply,
};
use std::{
    any::Any,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone)]
pub(crate) struct AgentEvalSite {
    pub(crate) name: Symbol,
    pub(crate) manifest: AgentManifest,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) capabilities: Vec<sim_kernel::CapabilityName>,
}

#[derive(Clone)]
pub(crate) struct AgentDispatchSite {
    pub(crate) state: Arc<Mutex<AgentState>>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
}

impl EvalSite for AgentEvalSite {
    fn site_kind(&self) -> &'static str {
        "agent"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: OnceLock<ServerAddress> = OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
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
            return Err(Error::Eval(format!(
                "agent {} only answers request frames",
                self.name
            )));
        }
        let consistency = frame.envelope.consistency;
        notify_recorders(cx, &self.manifest.recorders, &frame)?;
        let mut expr = eval_request_from_frame(cx, &frame)?.expr;
        if let Some(router) = &self.manifest.router {
            let _ = evaluate_component_expr(cx, router, expr.clone())?;
        }
        for retriever in &self.manifest.retrievers {
            expr = evaluate_component_expr(cx, retriever, expr)?;
        }
        if let Some(planner) = &self.manifest.planner {
            expr = evaluate_component_expr(cx, planner, expr)?;
        }
        if let Some(sandbox) = &self.manifest.sandbox {
            expr = evaluate_component_expr(cx, sandbox, expr)?;
        }
        if is_model_request(&expr) && !self.manifest.runners.is_empty() {
            expr = inject_manifest_tools(cx, expr, &self.manifest.tools)?;
            expr = evaluate_component_expr(cx, &self.manifest.runners[0], expr)?;
        } else {
            if !self.manifest.tools.is_empty() {
                let mut outputs = Vec::new();
                for tool in &self.manifest.tools {
                    outputs.push(evaluate_component_expr(cx, tool, expr.clone())?);
                }
                expr = if outputs.len() == 1 {
                    outputs.remove(0)
                } else {
                    Expr::List(outputs)
                };
            }
            if let Some(judge) = &self.manifest.judge {
                expr = evaluate_component_expr(cx, judge, expr)?;
            }
            if let Some(persona) = &self.manifest.persona {
                expr = evaluate_component_expr(cx, persona, expr)?;
            }
            if let Some(voice) = &self.manifest.voice {
                expr = evaluate_component_expr(cx, voice, expr)?;
            }
        }
        let reply_value = expr_to_value(cx, &expr)?;
        let diagnostics = cx.take_diagnostics();
        let reply = server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value: reply_value,
                diagnostics,
                trace: None,
            },
            consistency,
        )?;
        notify_recorders(cx, &self.manifest.recorders, &reply)?;
        Ok(reply)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for AgentDispatchSite {
    fn site_kind(&self) -> &'static str {
        "agent"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let runtime_site = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?
            .runtime_site
            .clone();
        runtime_site.answer(cx, frame)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Object for Agent {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<agent {}>", self.name))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for Agent {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(sim_kernel::ClassId(0), Symbol::qualified("agent", "Agent"))
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        let manifest = self.manifest_clone()?;
        let address_value = match &state.address {
            Some(address) => address.as_value(cx)?,
            None => cx.factory().symbol(Symbol::new("local"))?,
        };
        let components = cx.factory().list(collect_agent_components(&manifest))?;
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("agent"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.to_string())?),
            (Symbol::new("name"), cx.factory().symbol(self.name.clone())?),
            (Symbol::new("components"), components),
            (Symbol::new("address"), address_value),
        ])
    }
}

impl Object for AgentFabric {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<swarm {}>", self.name))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for AgentFabric {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("swarm", "AgentFabric"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let status = swarm_status_value_for_table(cx, self)?;
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("swarm"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.to_string())?),
            (Symbol::new("name"), cx.factory().symbol(self.name.clone())?),
            (
                Symbol::new("members"),
                cx.factory().list(self.members.clone())?,
            ),
            (Symbol::new("status"), status),
        ])
    }
    fn as_eval_fabric(&self) -> Option<&dyn sim_kernel::EvalFabric> {
        Some(self)
    }
}

impl Object for LoopbackStream {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<agent-stream>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for LoopbackStream {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(sim_kernel::ClassId(0), Symbol::qualified("agent", "Stream"))
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        let mut chunks = Vec::new();
        for value in self.handle.buffered_values()? {
            chunks.push(value.object().as_expr(cx)?);
        }
        Ok(Expr::List(chunks))
    }
    fn as_stream(&self) -> Option<&dyn sim_kernel::Stream> {
        Some(self.handle.as_ref())
    }
}

impl EvalSite for RuntimeValueSite {
    fn site_kind(&self) -> &'static str {
        self.kind
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if let Some(agent) = self.value.object().downcast_ref::<Agent>() {
            return agent.site()?.answer(cx, frame);
        }
        if let Some(fabric) = self.value.object().downcast_ref::<AgentFabric>() {
            return fabric.answer(cx, frame);
        }
        Err(Error::TypeMismatch {
            expected: "agent or swarm value",
            found: "non-routable value",
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn notify_recorders(cx: &mut Cx, recorders: &[Value], frame: &ServerFrame) -> Result<()> {
    for recorder in recorders {
        let site = site_from_value(recorder)?;
        let mut notify = frame.clone();
        notify.kind = FrameKind::Notify;
        let _ = site.answer(cx, notify)?;
    }
    Ok(())
}

pub(crate) fn evaluate_component_expr(cx: &mut Cx, value: &Value, expr: Expr) -> Result<Expr> {
    if let Some(connection) = connection_from_value(value) {
        return connection
            .request(cx, expr, None, Vec::new())?
            .object()
            .as_expr(cx);
    }
    let site = site_from_value(value)?;
    let request = ServerFrame::from_expr(
        cx,
        first_codec(site.codecs()),
        FrameKind::Request,
        &expr,
        sim_kernel::Consistency::LocalFirst,
        Vec::new(),
        false,
    )?;
    let reply = site.answer(cx, request)?;
    eval_reply_from_frame(cx, &reply)?
        .value
        .object()
        .as_expr(cx)
}

#[allow(dead_code)]
fn _assert_types(
    _: &Tool,
    _: &AgentComponent,
    _: &WorkingMemory,
    _: &FileMemory,
    _: &VectorMemory,
    _: &BlackboardMemory,
    _: &PersonaMemory,
) {
}
