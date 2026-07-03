use sim_kernel::{AbiVersion, Lib, LibManifest, LibTarget, Linker, Result, Symbol, Version};

use crate::{MCP_LIB_ID, McpFunction, ops::McpFunctionKind};

/// Loadable library that registers the MCP method functions in a runtime.
pub struct McpLib;

impl Lib for McpLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: manifest_name(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: crate::ops::mcp_exports(),
        }
    }

    fn load(&self, cx: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for kind in [
            McpFunctionKind::Handle,
            McpFunctionKind::Initialize,
            McpFunctionKind::Tools,
            McpFunctionKind::Call,
            McpFunctionKind::Resources,
            McpFunctionKind::Read,
            McpFunctionKind::Prompts,
            McpFunctionKind::GetPrompt,
            #[cfg(feature = "sampling")]
            McpFunctionKind::SamplingRunner,
            McpFunctionKind::Health,
        ] {
            let function = McpFunction::value(kind);
            linker.function_value(function.symbol(), cx.factory().opaque(function)?)?;
        }
        Ok(())
    }
}

/// Returns the symbol under which [`McpLib`] is registered.
pub fn manifest_name() -> Symbol {
    Symbol::new(MCP_LIB_ID)
}
