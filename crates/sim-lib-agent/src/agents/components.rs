use super::model::AgentManifest;
use crate::{
    AgentComponent, BlackboardMemory, FileMemory, PersonaMemory, Tool, VectorMemory, WorkingMemory,
    component_kind_symbol,
};
use sim_kernel::{Cx, Error, Result, Symbol, Value};

pub(crate) fn first_codec(codecs: &[Symbol]) -> Symbol {
    codecs
        .first()
        .cloned()
        .unwrap_or_else(|| Symbol::qualified("codec", "binary"))
}

pub(crate) fn collect_agent_components(manifest: &AgentManifest) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(value) = &manifest.conduct {
        out.push(value.clone());
    }
    out.extend(manifest.runners.clone());
    out.extend(manifest.tools.clone());
    out.extend(manifest.memories.clone());
    if let Some(value) = &manifest.planner {
        out.push(value.clone());
    }
    if let Some(value) = &manifest.judge {
        out.push(value.clone());
    }
    if let Some(value) = &manifest.router {
        out.push(value.clone());
    }
    if let Some(value) = &manifest.persona {
        out.push(value.clone());
    }
    out.extend(manifest.retrievers.clone());
    if let Some(value) = &manifest.sandbox {
        out.push(value.clone());
    }
    out.extend(manifest.recorders.clone());
    if let Some(value) = &manifest.voice {
        out.push(value.clone());
    }
    if let Some(value) = &manifest.topology {
        out.push(value.clone());
    }
    out.extend(manifest.extras.values().cloned());
    out
}

pub(crate) fn component_name(value: &Value) -> Result<Symbol> {
    if let Some(tool) = value.object().downcast_ref::<Tool>() {
        return Ok(tool.symbol.clone());
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        return Ok(component.symbol.clone());
    }
    if let Some(memory) = value.object().downcast_ref::<WorkingMemory>() {
        return Ok(memory.symbol.clone());
    }
    if let Some(memory) = value.object().downcast_ref::<FileMemory>() {
        return Ok(memory.symbol.clone());
    }
    if let Some(memory) = value.object().downcast_ref::<VectorMemory>() {
        return Ok(memory.symbol.clone());
    }
    if let Some(memory) = value.object().downcast_ref::<BlackboardMemory>() {
        return Ok(memory.symbol.clone());
    }
    if let Some(memory) = value.object().downcast_ref::<PersonaMemory>() {
        return Ok(memory.symbol.clone());
    }
    Err(Error::TypeMismatch {
        expected: "component",
        found: "non-component",
    })
}

pub(crate) fn component_kind_matches(cx: &mut Cx, value: &Value, kind: &str) -> Result<bool> {
    let actual = if value.object().downcast_ref::<Tool>().is_some() {
        "tool".to_owned()
    } else if value.object().downcast_ref::<WorkingMemory>().is_some()
        || value.object().downcast_ref::<FileMemory>().is_some()
        || value.object().downcast_ref::<VectorMemory>().is_some()
        || value.object().downcast_ref::<BlackboardMemory>().is_some()
        || value.object().downcast_ref::<PersonaMemory>().is_some()
    {
        "memory".to_owned()
    } else if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        component_kind_symbol(&component.kind).to_string()
    } else {
        format!("{:?}", value.object().as_expr(cx)?)
    };
    Ok(actual == kind)
}
