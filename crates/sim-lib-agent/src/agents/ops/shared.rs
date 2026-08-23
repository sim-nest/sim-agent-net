use crate::{
    AgentComponent, BlackboardMemory, FileMemory, PersonaMemory, Tool, VectorMemory, WorkingMemory,
    component_kind_symbol,
};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{Connection, ServerAddress};
use std::collections::HashMap;

use super::super::first_codec;
use super::super::model::{Agent, agent_from_value, connection_from_value, site_from_value};

pub(super) fn values_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<Value>> {
    let Some(value) = options.get(key) else {
        return Ok(Vec::new());
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => {
            items.into_iter().map(|expr| cx.eval_expr(expr)).collect()
        }
        _ => Ok(vec![value.clone()]),
    }
}

pub(super) fn optional_value(options: &HashMap<String, Value>, key: &str) -> Option<Value> {
    options.get(key).cloned()
}

pub(crate) fn agent_connection_for_value(value: Value) -> Result<Connection> {
    if let Some(connection) = connection_from_value(&value) {
        return Ok(connection.clone());
    }
    if let Ok(agent) = agent_from_value(&value) {
        let state = agent
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        let default_codec = state.default_codec.clone();
        let supported_codecs = state.supported_codecs.clone();
        drop(state);
        return Connection::with_session(
            ServerAddress::Local,
            default_codec,
            supported_codecs,
            agent.site()?,
            None,
            agent.policy.clone(),
        );
    }
    let site = site_from_value(&value)?;
    Connection::with_session(
        ServerAddress::Local,
        first_codec(site.codecs()),
        site.codecs().to_vec(),
        site,
        None,
        sim_lib_server::IsolationPolicy::default(),
    )
}

pub(crate) fn wire_step_connection(step: Value, role_tag: &str) -> Result<Connection> {
    let mut role = None;
    if role_tag == "auto" {
        if let Some(connection) = connection_from_value(&step) {
            role = connection.role().cloned();
        } else if step.object().downcast_ref::<Tool>().is_some() {
            role = Some(Symbol::new("tool"));
        } else if let Some(component) = step.object().downcast_ref::<AgentComponent>() {
            role = Some(component_kind_symbol(&component.kind));
        }
    }
    let connection = agent_connection_for_value(step)?;
    Connection::with_session(
        connection.address().clone(),
        connection.default_codec().clone(),
        connection.supported_codecs().to_vec(),
        connection.site().clone(),
        role,
        connection.session().isolation.clone(),
    )
}

pub(super) fn agent_make_expr(cx: &mut Cx, agent: &Agent) -> Result<Expr> {
    let manifest = agent.manifest_clone()?;
    let mut args = vec![
        Expr::Symbol(Symbol::new(":name")),
        Expr::Symbol(agent.name.clone()),
    ];
    if let Some(conduct) = &manifest.conduct {
        args.push(Expr::Symbol(Symbol::new(":conduct")));
        args.push(component_to_expr(cx, conduct)?);
    }
    if !manifest.runners.is_empty() {
        args.push(Expr::Symbol(Symbol::new(":runners")));
        args.push(Expr::List(
            manifest
                .runners
                .iter()
                .map(|value| component_to_expr(cx, value))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    if !manifest.tools.is_empty() {
        args.push(Expr::Symbol(Symbol::new(":tools")));
        args.push(Expr::List(
            manifest
                .tools
                .iter()
                .map(|value| component_to_expr(cx, value))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    if let Some(planner) = &manifest.planner {
        args.push(Expr::Symbol(Symbol::new(":planner")));
        args.push(component_to_expr(cx, planner)?);
    }
    if let Some(judge) = &manifest.judge {
        args.push(Expr::Symbol(Symbol::new(":judge")));
        args.push(component_to_expr(cx, judge)?);
    }
    if let Some(persona) = &manifest.persona {
        args.push(Expr::Symbol(Symbol::new(":persona")));
        args.push(component_to_expr(cx, persona)?);
    }
    if !manifest.recorders.is_empty() {
        args.push(Expr::Symbol(Symbol::new(":recorders")));
        args.push(Expr::List(
            manifest
                .recorders
                .iter()
                .map(|value| component_to_expr(cx, value))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    Ok(Expr::Call {
        operator: Box::new(Expr::Symbol(Symbol::qualified("agent", "make"))),
        args,
    })
}

fn component_to_expr(cx: &mut Cx, value: &Value) -> Result<Expr> {
    if let Some(tool) = value.object().downcast_ref::<Tool>() {
        return Ok(Expr::Symbol(tool.symbol.clone()));
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        let mut args = Vec::new();
        for (key, value) in &component.spec {
            if matches!(value, Expr::Nil) {
                continue;
            }
            args.push(Expr::Symbol(Symbol::new(format!(":{}", key.name))));
            args.push(value.clone());
        }
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(component.symbol.clone())),
            args,
        });
    }
    if value.object().downcast_ref::<WorkingMemory>().is_some() {
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "working"))),
            args: Vec::new(),
        });
    }
    if let Some(memory) = value.object().downcast_ref::<FileMemory>() {
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "file"))),
            args: vec![Expr::String(memory.path.display().to_string())],
        });
    }
    if let Some(memory) = value.object().downcast_ref::<VectorMemory>() {
        let args = memory
            .path
            .as_ref()
            .map(|path| {
                vec![
                    Expr::Symbol(Symbol::new(":path")),
                    Expr::String(path.display().to_string()),
                ]
            })
            .unwrap_or_default();
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "vector"))),
            args,
        });
    }
    if let Some(memory) = value.object().downcast_ref::<BlackboardMemory>() {
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "blackboard"))),
            args: vec![Expr::String(memory.board.clone())],
        });
    }
    if let Some(memory) = value.object().downcast_ref::<PersonaMemory>() {
        let args = memory
            .path
            .as_ref()
            .map(|path| {
                vec![
                    Expr::Symbol(Symbol::new(":path")),
                    Expr::String(path.display().to_string()),
                ]
            })
            .unwrap_or_default();
        return Ok(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "persona"))),
            args,
        });
    }
    value.object().as_expr(cx)
}
