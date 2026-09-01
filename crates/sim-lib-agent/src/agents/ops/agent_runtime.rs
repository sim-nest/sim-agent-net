use crate::agents::trace::{
    trace_entries, trace_role_matches, trace_timestamp_ms, trace_tool_matches,
};
use crate::agents::{audit_role_filter, parse_since_cutoff};
use crate::{
    Agent, AgentComponent, ComponentBackend, RecorderBackend, installed_codecs, lock_entries,
    parse_component_options, resolve_memory_backend, stringish_from_value, value_from_expr,
};
use sim_kernel::{Args, Cx, Error, Result, Symbol, Value};
use sim_lib_server::{EvalSite, Server, ServerAddress, ThreadMode};
use std::sync::Arc;

use super::super::model::{agent_from_value, register_started_agent};
use super::super::{build_agent_runtime_site, first_codec};

pub(crate) fn agent_server_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value] = args.values() else {
        return Err(Error::Eval("agent/server expects one agent".to_owned()));
    };
    let agent = agent_from_value(agent_value)?;
    let server = ensure_agent_server(agent)?;
    cx.factory().opaque(server)
}

pub(crate) fn agent_attach_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value, slot, attachment] = args.values() else {
        return Err(Error::Eval(
            "agent/attach expects an agent, slot, and component".to_owned(),
        ));
    };
    let agent = agent_from_value(agent_value)?;
    let slot = stringish_from_value(cx, slot.clone(), "agent/attach expects a slot name")?;
    {
        let mut manifest = agent
            .manifest
            .lock()
            .map_err(|_| Error::PoisonedLock("agent manifest"))?;
        match slot.as_str() {
            "conduct" => manifest.conduct = Some(attachment.clone()),
            "result-shape" => manifest.result_shape = Some(attachment.clone()),
            "tools" | "tool" => manifest.tools.push(attachment.clone()),
            "memories" | "memory" => manifest.memories.push(attachment.clone()),
            "retrievers" | "retriever" => manifest.retrievers.push(attachment.clone()),
            "recorders" | "recorder" => manifest.recorders.push(attachment.clone()),
            "planner" => manifest.planner = Some(attachment.clone()),
            "judge" => manifest.judge = Some(attachment.clone()),
            "router" => manifest.router = Some(attachment.clone()),
            "persona" => manifest.persona = Some(attachment.clone()),
            "sandbox" => manifest.sandbox = Some(attachment.clone()),
            "voice" => manifest.voice = Some(attachment.clone()),
            "topology" => manifest.topology = Some(attachment.clone()),
            other => {
                manifest
                    .extras
                    .insert(Symbol::new(other), attachment.clone());
            }
        }
    }
    restart_agent_runtime(cx, agent)?;
    Ok(agent_value.clone())
}

pub(crate) fn agent_audit_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value, rest @ ..] = args.values() else {
        return Err(Error::Eval("agent/audit expects one agent".to_owned()));
    };
    let agent = agent_from_value(agent_value)?;
    let options = parse_component_options(cx, Args::new(rest.to_vec()), "agent/audit")?;
    let since_cutoff = parse_since_cutoff(cx, options.get("since"))?;
    let role = audit_role_filter(
        cx,
        options.get("role"),
        "agent/audit :role expects a symbol",
    )?;
    let tool = audit_role_filter(
        cx,
        options.get("tool"),
        "agent/audit :tool expects a symbol",
    )?;
    let manifest = agent.manifest_clone()?;
    let mut entries = Vec::new();
    for recorder in manifest.recorders {
        if let Some(component) = recorder.object().downcast_ref::<AgentComponent>()
            && let ComponentBackend::Recorder(
                RecorderBackend::Journal { entries: store, .. }
                | RecorderBackend::Audit { entries: store, .. }
                | RecorderBackend::Prometheus { entries: store, .. },
            ) = &component.backend
        {
            let filtered = lock_entries(store, "recorder entries")?
                .iter()
                .filter(|entry| {
                    since_cutoff.is_none_or(|cutoff| {
                        trace_timestamp_ms(entry).is_none_or(|timestamp| timestamp >= cutoff)
                    })
                })
                .filter(|entry| {
                    role.as_ref()
                        .is_none_or(|role| trace_role_matches(entry, role))
                })
                .filter(|entry| {
                    tool.as_ref()
                        .is_none_or(|tool| trace_tool_matches(entry, tool))
                })
                .map(|expr| value_from_expr(cx, expr))
                .collect::<Result<Vec<_>>>()?;
            entries.extend(filtered);
        }
    }
    cx.factory().list(entries)
}

pub(crate) fn agent_trace_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [task_id] = args.values() else {
        return Err(Error::Eval("agent/trace expects one task id".to_owned()));
    };
    let task_id = stringish_from_value(cx, task_id.clone(), "agent/trace expects a task id")?;
    cx.factory()
        .expr(sim_kernel::Expr::List(trace_entries(&task_id)?))
}

pub(super) fn restart_agent_runtime(cx: &mut Cx, agent: &Agent) -> Result<()> {
    let old_manifest = agent.manifest_clone()?;
    let memory_snapshots = old_manifest
        .memories
        .iter()
        .map(|memory| resolve_memory_backend(memory)?.snapshot(cx))
        .collect::<Result<Vec<_>>>()?;
    let manifest = agent.manifest_clone()?;
    let supported_codecs = installed_codecs(cx);
    let runtime_site = build_agent_runtime_site(&manifest, &supported_codecs, &agent.capabilities)?;
    let address = agent
        .state
        .lock()
        .map_err(|_| Error::PoisonedLock("agent state"))?
        .address
        .clone();
    let server = match address.clone() {
        Some(address) => Some(Arc::new(create_agent_server(
            agent,
            runtime_site.clone(),
            address,
            supported_codecs.clone(),
            first_codec(&supported_codecs),
        )?)),
        None => None,
    };
    {
        let mut state = agent
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        state.runtime_site = runtime_site;
        if let Some(server) = server {
            state.server = Some(server);
        }
        state.supported_codecs = supported_codecs;
        if !state.supported_codecs.contains(&state.default_codec) {
            state.default_codec = first_codec(&state.supported_codecs);
        }
    }
    for (memory, snapshot) in manifest.memories.iter().zip(memory_snapshots) {
        resolve_memory_backend(memory)?.restore(cx, snapshot)?;
    }
    if agent
        .state
        .lock()
        .map_err(|_| Error::PoisonedLock("agent state"))?
        .address
        .is_some()
    {
        register_started_agent(agent)?;
    }
    Ok(())
}

fn ensure_agent_server(agent: &Agent) -> Result<Arc<Server>> {
    let mut state = agent
        .state
        .lock()
        .map_err(|_| Error::PoisonedLock("agent state"))?;
    if let Some(server) = &state.server {
        return Ok(server.clone());
    }
    let address = state.address.clone().unwrap_or(ServerAddress::Local);
    let server = Arc::new(create_agent_server(
        agent,
        state.runtime_site.clone(),
        address.clone(),
        state.supported_codecs.clone(),
        state.default_codec.clone(),
    )?);
    state.address = Some(address);
    state.server = Some(server.clone());
    Ok(server)
}

pub(super) fn create_agent_server(
    agent: &Agent,
    runtime_site: Arc<dyn EvalSite>,
    address: ServerAddress,
    supported_codecs: Vec<Symbol>,
    default_codec: Symbol,
) -> Result<Server> {
    Server::new(
        address,
        default_codec,
        supported_codecs,
        ThreadMode::Coop,
        agent.policy.clone(),
        Some(agent.name.clone()),
        runtime_site,
        Vec::new(),
    )
}
