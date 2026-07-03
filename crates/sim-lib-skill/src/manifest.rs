use sim_kernel::{AbiVersion, Lib, LibManifest, LibTarget, Linker, Result, Symbol, Version};

use crate::{SKILL_LIB_ID, SkillFunction, SkillRegistry, ops::SkillFunctionKind};

/// Loadable library that registers the `skill/*` operations and registry.
///
/// Loading `SkillLib` installs the [`SkillRegistry`] value and binds the
/// install, bind, list, card, and call functions, plus the feature-gated
/// audit, tool, runner, and serve surfaces.
pub struct SkillLib;

impl Lib for SkillLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: manifest_name(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: crate::ops::skill_exports(),
        }
    }

    fn load(&self, cx: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.value(
            crate::skill_registry_symbol(),
            cx.factory()
                .opaque(std::sync::Arc::new(SkillRegistry::default()))?,
        )?;
        for kind in [
            SkillFunctionKind::Install,
            SkillFunctionKind::Bind,
            SkillFunctionKind::List,
            SkillFunctionKind::Card,
            SkillFunctionKind::Call,
            #[cfg(any(feature = "cache", feature = "cassette"))]
            SkillFunctionKind::Audit,
            #[cfg(feature = "agent")]
            SkillFunctionKind::AsTool,
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpTools,
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpCall,
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTool,
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTools,
            #[cfg(feature = "runner")]
            SkillFunctionKind::AsRunner,
            #[cfg(feature = "serve")]
            SkillFunctionKind::ServeMcp,
        ] {
            let function = SkillFunction::value(kind);
            linker.function_value(function.symbol(), cx.factory().opaque(function)?)?;
        }
        Ok(())
    }
}

/// Returns the manifest identifier symbol for [`SkillLib`].
pub fn manifest_name() -> Symbol {
    Symbol::new(SKILL_LIB_ID)
}

pub use crate::ops::skill_exports;
