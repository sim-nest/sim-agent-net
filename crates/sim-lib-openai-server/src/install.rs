use sim_kernel::{Cx, Result};

use crate::manifest::OpenAiGatewayLib;

/// Installs the OpenAI gateway library into the context, idempotently.
///
/// Registers [`OpenAiGatewayLib`] via `install_once`, so repeated calls on the
/// same context are no-ops after the first.
pub fn install_openai_gateway_lib(cx: &mut Cx) -> Result<()> {
    sim_lib_core::install_once(cx, &OpenAiGatewayLib).map(|_| ())
}
