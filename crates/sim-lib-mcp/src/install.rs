use sim_kernel::{Cx, Result};

use crate::McpLib;

/// Installs [`McpLib`] into `cx`, loading it once if not already present.
pub fn install_mcp_lib(cx: &mut Cx) -> Result<()> {
    sim_lib_core::install_once(cx, &McpLib).map(|_| ())
}
