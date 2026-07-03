use super::runtime::{AgentFnKind, AgentFunction};
use sim_kernel::{Linker, Result};
use std::sync::Arc;

pub(crate) fn register_agent_functions(
    cx: &mut sim_kernel::LoadCx,
    linker: &mut Linker<'_>,
) -> Result<()> {
    for (symbol, kind) in AgentFnKind::all() {
        let function_id = linker.function(symbol.clone())?;
        let value = cx.factory().opaque(Arc::new(AgentFunction {
            symbol: symbol.clone(),
            kind: *kind,
        }))?;
        linker.bind_function_value(function_id, value)?;
    }
    Ok(())
}
