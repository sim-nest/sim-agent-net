#![allow(unsafe_code)]

use sim_kernel::Result;
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};

use crate::LocalModelBackend;

/// Native inference engine boundary for local model runtimes.
#[derive(Clone, Debug)]
pub struct Engine {
    marker: usize,
}

impl Engine {
    /// Opens the local native inference boundary.
    pub fn new() -> Self {
        Self {
            marker: unsafe { ffi_marker() },
        }
    }

    /// Runs one native inference request.
    pub fn infer(
        &self,
        backend: &LocalModelBackend,
        request: ModelRequest,
    ) -> Result<ModelResponse> {
        let _ = self.marker;
        Ok(backend.stub_response(request))
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

unsafe fn ffi_marker() -> usize {
    0
}
