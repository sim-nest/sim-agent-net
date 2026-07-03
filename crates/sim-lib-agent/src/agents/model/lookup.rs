use super::{Agent, AgentFabric, RuntimeValueSite};
use crate::{
    AgentComponent, BlackboardMemory, FileMemory, PersonaMemory, Tool, VectorMemory, WorkingMemory,
};
use sim_kernel::{Cx, Error, Result, Symbol, Value};
use sim_lib_server::{Connection, EvalSite, ResolvedAddress, ServerAddress};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone)]
struct StartedAgent {
    site: Arc<dyn EvalSite>,
    codecs: Vec<Symbol>,
    default_codec: Symbol,
}

fn started_agents() -> &'static Mutex<BTreeMap<String, StartedAgent>> {
    static STARTED: OnceLock<Mutex<BTreeMap<String, StartedAgent>>> = OnceLock::new();
    STARTED.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn site_from_value(value: &Value) -> Result<Arc<dyn EvalSite>> {
    if let Some(tool) = value.object().downcast_ref::<Tool>() {
        return Ok(Arc::new(tool.clone()));
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        return Ok(Arc::new(component.clone()));
    }
    if let Some(memory) = value.object().downcast_ref::<WorkingMemory>() {
        return Ok(Arc::new(memory.clone()));
    }
    if let Some(memory) = value.object().downcast_ref::<FileMemory>() {
        return Ok(Arc::new(memory.clone()));
    }
    if let Some(memory) = value.object().downcast_ref::<VectorMemory>() {
        return Ok(Arc::new(memory.clone()));
    }
    if let Some(memory) = value.object().downcast_ref::<BlackboardMemory>() {
        return Ok(Arc::new(memory.clone()));
    }
    if let Some(memory) = value.object().downcast_ref::<PersonaMemory>() {
        return Ok(Arc::new(memory.clone()));
    }
    if let Some(agent) = value.object().downcast_ref::<Agent>() {
        return agent.site();
    }
    if let Some(fabric) = value.object().downcast_ref::<AgentFabric>() {
        return Ok(Arc::new(RuntimeValueSite {
            value: value.clone(),
            address: ServerAddress::Local,
            codecs: fabric.codecs.clone(),
            kind: "swarm",
        }));
    }
    Err(Error::TypeMismatch {
        expected: "component, agent, or swarm",
        found: "non-site",
    })
}

pub(crate) fn agent_from_value(value: &Value) -> Result<&Agent> {
    value
        .object()
        .downcast_ref::<Agent>()
        .ok_or(Error::TypeMismatch {
            expected: "agent",
            found: "non-agent",
        })
}

pub(crate) fn fabric_from_value(value: &Value) -> Result<&AgentFabric> {
    value
        .object()
        .downcast_ref::<AgentFabric>()
        .ok_or(Error::TypeMismatch {
            expected: "swarm",
            found: "non-swarm",
        })
}

pub(crate) fn connection_from_value(value: &Value) -> Option<&Connection> {
    value.object().downcast_ref::<Connection>()
}

pub(crate) fn register_started_agent(agent: &Agent) -> Result<()> {
    let state = agent
        .state
        .lock()
        .map_err(|_| Error::PoisonedLock("agent state"))?;
    if state.address.is_none() {
        return Ok(());
    }
    let codecs = state.supported_codecs.clone();
    let default_codec = state.default_codec.clone();
    drop(state);
    let mut started = started_agents()
        .lock()
        .map_err(|_| Error::PoisonedLock("started agents"))?;
    started.insert(
        agent.name.to_string(),
        StartedAgent {
            site: agent.site()?,
            codecs,
            default_codec,
        },
    );
    Ok(())
}

pub(crate) fn resolve_agent_address(
    _cx: &mut Cx,
    address: &ServerAddress,
    offered: &[Symbol],
) -> Result<Option<ResolvedAddress>> {
    let ServerAddress::Agent { agent } = address else {
        return Ok(None);
    };
    let started = started_agents()
        .lock()
        .map_err(|_| Error::PoisonedLock("started agents"))?;
    let Some(started) = started.get(agent) else {
        return Ok(None);
    };
    let selected = offered
        .iter()
        .find(|codec| started.codecs.iter().any(|supported| supported == *codec))
        .cloned()
        .unwrap_or_else(|| started.default_codec.clone());
    Ok(Some(ResolvedAddress {
        site: started.site.clone(),
        selected_codec: selected,
        supported_codecs: started.codecs.clone(),
    }))
}
