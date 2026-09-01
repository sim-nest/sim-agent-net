pub(crate) fn build_agent_runtime_site(
    manifest: &AgentManifest,
    codecs: &[Symbol],
    capabilities: &[CapabilityName],
) -> Result<Arc<dyn EvalSite>> {
    validate_required_roles(manifest)?;
    let conduct_site = manifest
        .conduct
        .as_ref()
        .map(crate::agents::site_from_value)
        .transpose()?;
    let connections = pipeline_connections_from_manifest(manifest)?;
    let pipeline_address = ServerAddress::Pipeline {
        steps: connections
            .iter()
            .map(|step| step.address().clone())
            .collect(),
    };
    let inner: Arc<dyn EvalSite> = if let Some(site) = conduct_site {
        site
    } else if connections.is_empty() {
        Arc::new(IdentityEvalSite {
            codecs: codecs.to_vec(),
        })
    } else {
        Arc::new(PipelineEvalSite::new(
            pipeline_address,
            codecs.to_vec(),
            connections,
        ))
    };
    Ok(Arc::new(AgentRuntimeSite::new(
        AgentRuntimeConfig {
            capabilities: capabilities.to_vec(),
            recorders: manifest.recorders.clone(),
            model_runners: manifest.runners.clone(),
            model_tools: manifest.tools.clone(),
            conduct: manifest.conduct.clone(),
            budget: manifest.budget,
            result_shape: manifest.result_shape.clone(),
        },
        inner,
    )))
}

fn narrow_run_frame(
    cx: &mut Cx,
    frame: ServerFrame,
    budget: Option<usize>,
    result_shape: Option<&Value>,
) -> Result<ServerFrame> {
    let mut request = eval_request_from_frame(cx, &frame)?;
    request.answer_limit = match (request.answer_limit, budget) {
        (Some(caller), Some(default)) => Some(caller.min(default)),
        (caller @ Some(_), None) => caller,
        (None, default) => default,
    };
    if request.result_shape.is_none() {
        request.result_shape = result_shape.cloned();
    }
    let mut narrowed = server_frame_from_request(cx, &frame.codec, request)?;
    narrowed.msg_id = frame.msg_id;
    narrowed.correlate = frame.correlate;
    narrowed.envelope = frame.envelope;
    Ok(narrowed)
}

fn select_runner<'a>(
    _cx: &mut Cx,
    runners: &'a [Value],
    requested: Option<&Symbol>,
) -> Result<&'a Value> {
    if runners.is_empty() {
        return Err(Error::Eval("agent has no model runner".to_owned()));
    }
    if let Some(role) = requested {
        let label = role.to_string();
        if let Some(index) = label
            .strip_prefix("runner-")
            .or_else(|| label.strip_prefix("provider-"))
            .and_then(|value| value.parse::<usize>().ok())
        {
            return runners
                .get(index)
                .ok_or_else(|| Error::Eval(format!("agent runner role {role} is unresolved")));
        }
        if let Some(value) = runners.iter().find(|value| {
            component_name(value).is_ok_and(|name| name == *role || name.name == role.name)
        }) {
            return Ok(value);
        }
    }
    // Provider_4 is the durable compatibility seat. Existing manifests that
    // predate named seats retain their explicitly indexed runner-0 binding.
    runners
        .iter()
        .find(|value| component_name(value).is_ok_and(|name| name.name.as_ref() == "Provider_4"))
        .or_else(|| runners.first())
        .ok_or_else(|| Error::Eval("agent has no compatible runner seat".to_owned()))
}

fn validate_required_roles(manifest: &AgentManifest) -> Result<()> {
    let Some(conduct) = &manifest.conduct else {
        return Ok(());
    };
    let expr = conduct.object().as_expr(&mut Cx::new(
        Arc::new(sim_kernel::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    ))?;
    let required = required_roles_from_expr(&expr);
    for role in required {
        if resolve_manifest_role(manifest, &role).is_none() {
            return Err(Error::Eval(format!(
                "agent conduct requires unresolved role {role}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn required_roles_from_expr(expr: &sim_kernel::Expr) -> Vec<Symbol> {
    let sim_kernel::Expr::Map(entries) = expr else {
        return Vec::new();
    };
    entries
        .iter()
        .find_map(|(key, value)| match (key, value) {
            (sim_kernel::Expr::Symbol(key), sim_kernel::Expr::List(items))
                if matches!(key.name.as_ref(), "required-roles" | "required_roles") =>
            {
                Some(
                    items
                        .iter()
                        .filter_map(|item| match item {
                            sim_kernel::Expr::Symbol(role) => Some(role.clone()),
                            _ => None,
                        })
                        .collect(),
                )
            }
            _ => None,
        })
        .unwrap_or_default()
}

pub(crate) fn conduct_id(cx: &mut Cx, manifest: &AgentManifest) -> Result<Symbol> {
    match &manifest.conduct {
        Some(conduct) => component_name(conduct).or_else(|_| {
            Ok(Symbol::new(format!(
                "conduct:{:?}",
                conduct.object().as_expr(cx)?
            )))
        }),
        None => Ok(Symbol::qualified("agent.conduct", "compatibility")),
    }
}

pub(crate) fn graph_fingerprint(cx: &mut Cx, manifest: &AgentManifest) -> Result<String> {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    conduct_id(cx, manifest)?.to_string().hash(&mut state);
    for (role, identity, _) in manifest_bindings(manifest) {
        role.to_string().hash(&mut state);
        identity.to_string().hash(&mut state);
    }
    Ok(format!("agent-graph:{:016x}", state.finish()))
}

/// Stable role/Card identity kept separately from its process-local value.
pub(crate) fn manifest_bindings(manifest: &AgentManifest) -> Vec<(Symbol, Symbol, Value)> {
    let mut out = Vec::new();
    let mut add = |role: Symbol, value: &Value| {
        let identity = component_name(value).unwrap_or_else(|_| role.clone());
        out.push((role, identity, value.clone()));
    };
    for (index, value) in manifest.runners.iter().enumerate() {
        add(Symbol::new(format!("provider-{index}")), value);
    }
    for (prefix, values) in [
        ("tool", &manifest.tools),
        ("memory", &manifest.memories),
        ("retriever", &manifest.retrievers),
        ("recorder", &manifest.recorders),
    ] {
        for (index, value) in values.iter().enumerate() {
            add(Symbol::new(format!("{prefix}-{index}")), value);
        }
    }
    for (role, value) in [
        ("planner", manifest.planner.as_ref()),
        ("judge", manifest.judge.as_ref()),
        ("router", manifest.router.as_ref()),
        ("persona", manifest.persona.as_ref()),
        ("sandbox", manifest.sandbox.as_ref()),
        ("voice", manifest.voice.as_ref()),
        ("topology", manifest.topology.as_ref()),
    ] {
        if let Some(value) = value {
            add(Symbol::new(role), value);
        }
    }
    for (role, value) in &manifest.extras {
        add(role.clone(), value);
    }
    out
}

fn resolve_manifest_role<'a>(manifest: &'a AgentManifest, role: &Symbol) -> Option<&'a Value> {
    let name = role.name.as_ref();
    let indexed = |prefix: &str, values: &'a [Value]| {
        name.strip_prefix(prefix)
            .and_then(|index| index.trim_start_matches(['-', '_']).parse::<usize>().ok())
            .and_then(|index| values.get(index))
    };
    let direct = indexed("runner", &manifest.runners)
        .or_else(|| indexed("provider", &manifest.runners))
        .or_else(|| indexed("tool", &manifest.tools))
        .or_else(|| indexed("memory", &manifest.memories))
        .or_else(|| indexed("retriever", &manifest.retrievers))
        .or_else(|| indexed("recorder", &manifest.recorders))
        .or_else(|| match name {
            "planner" => manifest.planner.as_ref(),
            "judge" => manifest.judge.as_ref(),
            "router" => manifest.router.as_ref(),
            "persona" => manifest.persona.as_ref(),
            "sandbox" => manifest.sandbox.as_ref(),
            "voice" => manifest.voice.as_ref(),
            "topology" => manifest.topology.as_ref(),
            _ => manifest.extras.get(role).or_else(|| {
                manifest
                    .extras
                    .iter()
                    .find_map(|(key, value)| (key.name == role.name).then_some(value))
            }),
        });
    direct.or_else(|| {
        manifest
            .runners
            .iter()
            .chain(&manifest.tools)
            .chain(&manifest.memories)
            .chain(manifest.retrievers.iter())
            .chain(manifest.recorders.iter())
            .find(|value| {
                component_name(value)
                    .is_ok_and(|identity| identity == *role || identity.name == role.name)
            })
    })
}

pub(crate) fn pipeline_connections_from_manifest(
    manifest: &AgentManifest,
) -> Result<Vec<Connection>> {
    let mut connections = Vec::new();
    if let Some(router) = &manifest.router {
        connections.push(sniffed_connection(
            router.clone(),
            router_tap_connection(router.clone())?,
            &manifest.recorders,
        )?);
    }
    for retriever in &manifest.retrievers {
        connections.push(sniffed_wire_connection(
            retriever.clone(),
            &manifest.recorders,
        )?);
    }
    if let Some(planner) = &manifest.planner {
        connections.push(sniffed_wire_connection(
            planner.clone(),
            &manifest.recorders,
        )?);
    }
    if let Some(sandbox) = &manifest.sandbox {
        connections.push(sniffed_wire_connection(
            sandbox.clone(),
            &manifest.recorders,
        )?);
    }
    for tool in &manifest.tools {
        connections.push(sniffed_wire_connection(tool.clone(), &manifest.recorders)?);
    }
    if let Some(judge) = &manifest.judge {
        connections.push(sniffed_wire_connection(judge.clone(), &manifest.recorders)?);
    }
    if let Some(persona) = &manifest.persona {
        connections.push(sniffed_wire_connection(
            persona.clone(),
            &manifest.recorders,
        )?);
    }
    if let Some(voice) = &manifest.voice {
        connections.push(sniffed_wire_connection(voice.clone(), &manifest.recorders)?);
    }
    Ok(connections)
}

fn router_tap_connection(step: Value) -> Result<Connection> {
    let connection = wire_step_connection(step, "auto")?;
    Connection::with_session(
        connection.address().clone(),
        connection.default_codec().clone(),
        connection.supported_codecs().to_vec(),
        Arc::new(RouterTapSite {
            codecs: connection.supported_codecs().to_vec(),
            inner: connection.site().clone(),
        }),
        connection.role().cloned(),
        connection.session().isolation.clone(),
    )
}

fn sniffed_wire_connection(step: Value, recorders: &[Value]) -> Result<Connection> {
    sniffed_connection(step.clone(), wire_step_connection(step, "auto")?, recorders)
}

fn sniffed_connection(
    step: Value,
    connection: Connection,
    recorders: &[Value],
) -> Result<Connection> {
    if recorders.is_empty() {
        return Ok(connection);
    }
    let stage = component_name(&step)
        .map(|symbol| symbol.to_string())
        .unwrap_or_else(|_| {
            connection
                .role()
                .map(Symbol::to_string)
                .unwrap_or_else(|| "stage".to_owned())
        });
    let tool = crate::agents::trace::tool_symbol(&step);
    Connection::with_session(
        connection.address().clone(),
        connection.default_codec().clone(),
        connection.supported_codecs().to_vec(),
        Arc::new(RecorderSnifferSite {
            inner: connection.site().clone(),
            recorders: recorders.to_vec(),
            stage,
            tool,
        }),
        connection.role().cloned(),
        connection.session().isolation.clone(),
    )
}
