use super::shared::{
    agent_connection_for_value, agent_make_expr, optional_value, values_option,
    wire_step_connection,
};
use crate::{
    AGENT_REFLECT_CAPABILITY, AGENT_REPLACE_CAPABILITY, AGENT_SPAWN_CAPABILITY, BlackboardMemory,
    FileMemory, PersonaMemory, Tool, VectorMemory, WorkingMemory, capabilities_option,
    installed_codecs, parse_component_options, string_option, stringish_from_value,
    symbol_from_value, symbol_option,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Result, Symbol, Value};
use sim_lib_server::{
    BufferedStreamSink, Connection, PipelineEvalSite, ServerAddress, StreamHandle, StreamSink,
    server_frame_from_request,
};
use std::{collections::BTreeMap, sync::Arc};

use super::super::model::{
    Agent, AgentFabric, AgentManifest, LoopbackStream, agent_from_value, register_started_agent,
    resolve_agent_address, site_from_value,
};
use super::super::{
    build_agent_runtime_site, collect_agent_components, component_kind_matches, component_name,
    first_codec,
};
use super::agent_runtime::{create_agent_server, restart_agent_runtime};

pub(crate) fn agent_make_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(AGENT_SPAWN_CAPABILITY))?;
    let options = parse_component_options(cx, args, "agent/make")?;
    let name = symbol_option(cx, &options, "name", Symbol::new("agent"))?;
    let policy = match options.get("policy") {
        Some(value) => sim_lib_server::IsolationPolicy::from_expr(&value.object().as_expr(cx)?)?,
        None => sim_lib_server::IsolationPolicy::default(),
    };
    let manifest = AgentManifest {
        runners: values_option(cx, &options, "runners")?,
        tools: values_option(cx, &options, "tools")?,
        memories: values_option(cx, &options, "memories")?,
        planner: optional_value(&options, "planner"),
        judge: optional_value(&options, "judge"),
        router: optional_value(&options, "router"),
        persona: optional_value(&options, "persona"),
        retrievers: values_option(cx, &options, "retrievers")?,
        sandbox: optional_value(&options, "sandbox"),
        recorders: values_option(cx, &options, "recorders")?,
        voice: optional_value(&options, "voice"),
        topology: optional_value(&options, "topology"),
        extras: BTreeMap::new(),
    };
    let capabilities = capabilities_option(cx, &options, "capable")?;
    let agent = Agent::new(name, manifest, capabilities, policy, installed_codecs(cx));
    cx.factory().opaque(Arc::new(agent))
}

pub(crate) fn agent_start_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(AGENT_SPAWN_CAPABILITY))?;
    let [agent_value, rest @ ..] = args.values() else {
        return Err(Error::Eval(
            "agent/start expects an agent and optional key/value options".to_owned(),
        ));
    };
    let agent = agent_from_value(agent_value)?;
    let options = parse_component_options(cx, Args::new(rest.to_vec()), "agent/start")?;
    let address = match options.get("address") {
        Some(value) => ServerAddress::from_expr(&value.object().as_expr(cx)?)?,
        None => ServerAddress::Agent {
            agent: agent.name.to_string(),
        },
    };
    let selected_codec = match options.get("codec") {
        Some(value) => symbol_from_value(cx, value.clone(), "agent/start :codec expects a symbol")?,
        None => first_codec(&installed_codecs(cx)),
    };
    let supported_codecs = installed_codecs(cx);
    let manifest = agent.manifest_clone()?;
    let runtime_site = build_agent_runtime_site(&manifest, &supported_codecs, &agent.capabilities)?;
    let server = Arc::new(create_agent_server(
        agent,
        runtime_site.clone(),
        address.clone(),
        supported_codecs.clone(),
        selected_codec.clone(),
    )?);
    {
        let mut state = agent
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        state.address = Some(address);
        state.server = Some(server);
        state.default_codec = selected_codec;
        state.supported_codecs = supported_codecs;
        state.runtime_site = runtime_site;
    }
    register_started_agent(agent)?;
    Ok(agent_value.clone())
}

pub(crate) fn agent_connect_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [target, rest @ ..] = args.values() else {
        return Err(Error::Eval("agent/connect expects a target".to_owned()));
    };
    let mut codec = None;
    if !rest.len().is_multiple_of(2) {
        return Err(Error::Eval(
            "agent/connect options must be key/value pairs".to_owned(),
        ));
    }
    for pair in rest.chunks(2) {
        let key = crate::keyword(&pair[0].object().as_expr(cx)?)?;
        if key == "codec" {
            codec = Some(symbol_from_value(
                cx,
                pair[1].clone(),
                "agent/connect :codec expects a symbol",
            )?);
        }
    }
    let connection = if let Ok(agent) = agent_from_value(target) {
        let state = agent
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        let selected_codec = codec.unwrap_or_else(|| state.default_codec.clone());
        let supported_codecs = state.supported_codecs.clone();
        drop(state);
        Connection::with_session(
            ServerAddress::Local,
            selected_codec,
            supported_codecs,
            agent.site()?,
            None,
            agent.policy.clone(),
        )?
    } else {
        let address = ServerAddress::from_expr(&target.object().as_expr(cx)?)?;
        if let Some(resolved) = resolve_agent_address(cx, &address, &installed_codecs(cx))? {
            Connection::with_session(
                address,
                codec.unwrap_or(resolved.selected_codec),
                resolved.supported_codecs,
                resolved.site,
                None,
                sim_lib_server::IsolationPolicy::default(),
            )?
        } else {
            let site = site_from_value(target)?;
            Connection::with_session(
                address,
                codec.unwrap_or_else(|| first_codec(site.codecs())),
                site.codecs().to_vec(),
                site,
                None,
                sim_lib_server::IsolationPolicy::default(),
            )?
        }
    };
    cx.factory().opaque(Arc::new(connection))
}

pub(crate) fn agent_call_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [target, expr, ..] = args.values() else {
        return Err(Error::Eval(
            "agent/call expects an agent and an expression".to_owned(),
        ));
    };
    let connection = agent_connection_for_value(target.clone())?;
    let request_expr = expr.object().as_expr(cx)?;
    connection.request(cx, request_expr, None, Vec::new())
}

pub(crate) fn agent_stream_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [target, expr, ..] = args.values() else {
        return Err(Error::Eval(
            "agent/stream expects an agent and an expression".to_owned(),
        ));
    };
    let connection = agent_connection_for_value(target.clone())?;
    let request_expr = expr.object().as_expr(cx)?;
    let consistency =
        if connection.address().is_remote_like() || connection.site().address().is_remote_like() {
            sim_kernel::Consistency::RemoteOnly
        } else {
            sim_kernel::Consistency::LocalFirst
        };
    let mut frame = server_frame_from_request(
        cx,
        connection.default_codec(),
        sim_kernel::EvalRequest {
            expr: request_expr,
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: true,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency,
            trace: false,
        },
    )?;
    frame.envelope.role = connection.role().cloned();
    let handle = Arc::new(StreamHandle::default());
    let mut sink = BufferedStreamSink::new(handle.clone());
    connection.site().stream(cx, frame, &mut sink)?;
    sink.end(cx)?;
    let stream = LoopbackStream::new(handle);
    cx.factory().opaque(Arc::new(stream))
}

pub(crate) fn agent_component_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value, kind_value, maybe_name @ ..] = args.values() else {
        return Err(Error::Eval(
            "agent/component expects an agent, kind, and optional name".to_owned(),
        ));
    };
    let agent = agent_from_value(agent_value)?;
    let kind = stringish_from_value(cx, kind_value.clone(), "agent/component expects a kind")?;
    let name = maybe_name
        .first()
        .map(|value| stringish_from_value(cx, value.clone(), "agent/component expects a name"))
        .transpose()?;
    for component in collect_agent_components(&agent.manifest_clone()?) {
        if component_kind_matches(cx, &component, &kind)?
            && name.as_ref().is_none_or(|expected| {
                component_name(&component)
                    .map(|symbol| symbol.to_string() == *expected)
                    .unwrap_or(false)
            })
        {
            return Ok(component);
        }
    }
    Err(Error::Eval(format!(
        "agent {} has no component for kind {}",
        agent.name, kind
    )))
}

pub(crate) fn agent_components_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value] = args.values() else {
        return Err(Error::Eval("agent/components expects one agent".to_owned()));
    };
    let agent = agent_from_value(agent_value)?;
    let components = collect_agent_components(&agent.manifest_clone()?);
    cx.factory().list(components)
}

pub(crate) fn agent_replace_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(AGENT_REPLACE_CAPABILITY))?;
    let [agent_value, slot, replacement] = args.values() else {
        return Err(Error::Eval(
            "agent/replace expects an agent, slot, and component".to_owned(),
        ));
    };
    let agent = agent_from_value(agent_value)?;
    let slot = stringish_from_value(cx, slot.clone(), "agent/replace expects a slot name")?;
    {
        let mut manifest = agent
            .manifest
            .lock()
            .map_err(|_| Error::PoisonedLock("agent manifest"))?;
        match slot.as_str() {
            "runners" | "runner" => manifest.runners = vec![replacement.clone()],
            "planner" => manifest.planner = Some(replacement.clone()),
            "judge" => manifest.judge = Some(replacement.clone()),
            "router" => manifest.router = Some(replacement.clone()),
            "persona" => manifest.persona = Some(replacement.clone()),
            "sandbox" => manifest.sandbox = Some(replacement.clone()),
            "voice" => manifest.voice = Some(replacement.clone()),
            "topology" => manifest.topology = Some(replacement.clone()),
            "tools" | "tool" => manifest.tools = vec![replacement.clone()],
            "memories" | "memory" => manifest.memories = vec![replacement.clone()],
            "retrievers" | "retriever" => manifest.retrievers = vec![replacement.clone()],
            "recorders" | "recorder" => manifest.recorders = vec![replacement.clone()],
            other => {
                manifest
                    .extras
                    .insert(Symbol::new(other), replacement.clone());
            }
        }
    }
    restart_agent_runtime(cx, agent)?;
    Ok(agent_value.clone())
}

pub(crate) fn agent_restart_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value] = args.values() else {
        return Err(Error::Eval("agent/restart expects one agent".to_owned()));
    };
    let agent = agent_from_value(agent_value)?;
    restart_agent_runtime(cx, agent)?;
    Ok(agent_value.clone())
}

pub(crate) fn agent_derive_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(AGENT_SPAWN_CAPABILITY))?;
    let [agent_value, rest @ ..] = args.values() else {
        return Err(Error::Eval("agent/derive expects an agent".to_owned()));
    };
    let source = agent_from_value(agent_value)?;
    let options = parse_component_options(cx, Args::new(rest.to_vec()), "agent/derive")?;
    let mut manifest = source.manifest_clone()?;
    if let Some(persona) = options.get("persona") {
        manifest.persona = Some(persona.clone());
    }
    let name = match options.get("name") {
        Some(value) => symbol_from_value(cx, value.clone(), "agent/derive :name expects a symbol")?,
        None => Symbol::new(format!("{}-derived", source.name)),
    };
    let derived = Agent::new(
        name,
        manifest,
        source.capabilities.clone(),
        source.policy.clone(),
        installed_codecs(cx),
    );
    cx.factory().opaque(Arc::new(derived))
}

pub(crate) fn agent_lisp_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [agent_value] = args.values() else {
        return Err(Error::Eval("agent/lisp expects one agent".to_owned()));
    };
    let agent = agent_from_value(agent_value)?;
    let expr = agent_make_expr(cx, agent)?;
    cx.factory().expr(expr)
}

pub(crate) fn agent_reflect_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(AGENT_REFLECT_CAPABILITY))?;
    let [agent_value] = args.values() else {
        return Err(Error::Eval("agent/reflect expects one agent".to_owned()));
    };
    agent_value.object().as_table(cx)
}

pub(crate) fn agent_wire_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "agent/wire")?;
    let steps = values_option(cx, &options, "steps")?;
    let role_tag = string_option(cx, &options, "role-tag", "auto")?;
    let connections = steps
        .into_iter()
        .map(|step| wire_step_connection(step, &role_tag))
        .collect::<Result<Vec<_>>>()?;
    let codecs = installed_codecs(cx);
    let connection = Connection::with_session(
        ServerAddress::Pipeline {
            steps: connections
                .iter()
                .map(|step| step.address().clone())
                .collect(),
        },
        first_codec(&codecs),
        codecs.clone(),
        Arc::new(PipelineEvalSite::new(
            ServerAddress::Pipeline {
                steps: connections
                    .iter()
                    .map(|step| step.address().clone())
                    .collect(),
            },
            codecs.clone(),
            connections,
        )),
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?;
    cx.factory().opaque(Arc::new(connection))
}

#[allow(dead_code)]
fn _assert_types(
    _: &Tool,
    _: &WorkingMemory,
    _: &FileMemory,
    _: &VectorMemory,
    _: &BlackboardMemory,
    _: &PersonaMemory,
    _: &AgentFabric,
) {
}
