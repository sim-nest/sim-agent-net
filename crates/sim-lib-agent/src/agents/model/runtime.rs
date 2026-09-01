use super::super::collect_agent_components;
use super::types::AgentState;
use super::{
    Agent, AgentFabric, AgentManifest, LoopbackStream, RuntimeValueSite,
    swarm_status_value_for_table,
};
use sim_kernel::{ClassRef, Cx, Error, Expr, Object, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerAddress, ServerFrame};
use std::{
    any::Any,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone)]
pub(crate) struct AgentEvalSite {
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
        super::super::build_agent_runtime_site(&self.manifest, &self.codecs, &self.capabilities)?
            .answer(cx, frame)
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
        let conduct_id = super::super::conduct_id(cx, &manifest)?;
        let graph_fingerprint = super::super::graph_fingerprint(cx, &manifest)?;
        let required_roles = match &manifest.conduct {
            Some(conduct) => super::super::required_roles_from_expr(&conduct.object().as_expr(cx)?),
            None => vec![Symbol::new("provider-0")],
        };
        let required_roles = cx.factory().list(
            required_roles
                .into_iter()
                .map(|role| cx.factory().symbol(role))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let effective_capabilities = cx.factory().list(
            self.capabilities
                .iter()
                .map(|capability| cx.factory().symbol(capability.as_symbol()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let budget = match manifest.budget {
            Some(value) => cx.factory().string(value.to_string())?,
            None => cx.factory().nil()?,
        };
        let result_contract = manifest.result_shape.clone().unwrap_or(cx.factory().nil()?);
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("agent"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.to_string())?),
            (Symbol::new("name"), cx.factory().symbol(self.name.clone())?),
            (Symbol::new("components"), components),
            (Symbol::new("conduct-id"), cx.factory().symbol(conduct_id)?),
            (
                Symbol::new("graph-fingerprint"),
                cx.factory().string(graph_fingerprint)?,
            ),
            (Symbol::new("required-roles"), required_roles),
            (
                Symbol::new("effective-capabilities"),
                effective_capabilities,
            ),
            (Symbol::new("budget-defaults"), budget),
            (Symbol::new("result-contract"), result_contract),
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
