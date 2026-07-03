use std::sync::Arc;

use sim_kernel::{CapabilityName, Cx, Result, ShapeRef, Symbol, Value};
use sim_lib_server::ServerAddress;

use crate::tools::{Tool, register_tool};
use crate::util::installed_codecs;

/// Specification for projecting a runtime value into a local [`Tool`].
pub struct ToolSpec {
    /// Symbol the tool is registered and dispatched under.
    pub symbol: Symbol,
    /// Human-facing description of the tool.
    pub description: String,
    /// Shape the tool's arguments must match.
    pub args_shape: ShapeRef,
    /// Shape the tool's result is checked against, if any.
    pub result_shape: Option<ShapeRef>,
    /// Category symbol grouping the tool.
    pub category: Symbol,
    /// Capabilities the tool requires to run.
    pub capabilities: Vec<CapabilityName>,
    /// Callable value invoked when the tool runs.
    pub function: Value,
}

impl Tool {
    /// Builds a local tool from a [`ToolSpec`], binding installed codecs.
    pub fn local(cx: &mut Cx, spec: ToolSpec) -> Self {
        Self {
            symbol: spec.symbol,
            description: spec.description,
            args_shape: spec.args_shape,
            result_shape: spec.result_shape,
            category: spec.category,
            capabilities: spec.capabilities,
            function: spec.function,
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
        }
    }
}

/// Installs a tool into the registry, reusing any existing registration.
pub fn install_tool(cx: &mut Cx, tool: Arc<Tool>) -> Result<Value> {
    if let Some(existing) = cx.registry().value_by_symbol(&tool.symbol).cloned()
        && existing.object().downcast_ref::<Tool>().is_some()
    {
        return Ok(existing);
    }
    let value = cx.factory().opaque(tool.clone())?;
    register_tool(cx, tool, value.clone())?;
    Ok(value)
}
