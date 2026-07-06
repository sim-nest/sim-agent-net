use crate::runtime::{OpenAiFederation, OpenAiRunnerRegistry};

pub use super::execution_record::{
    GatewayRunExecution as ResponseExecution, GatewayRunIdGenerators as ResponseIdGenerators,
};

/// Bundles the eval targets a response run may dispatch to: the runner
/// registry and an optional federation surface for cross-node eval.
#[derive(Clone, Copy)]
pub struct ResponseRuntimeTargets<'a> {
    runners: &'a OpenAiRunnerRegistry,
    federation: Option<&'a OpenAiFederation>,
}

impl<'a> ResponseRuntimeTargets<'a> {
    /// Builds targets backed by the runner registry alone, with no federation.
    pub fn runners(runners: &'a OpenAiRunnerRegistry) -> Self {
        Self {
            runners,
            federation: None,
        }
    }

    /// Builds targets backed by both the runner registry and a federation
    /// surface.
    pub fn with_federation(
        runners: &'a OpenAiRunnerRegistry,
        federation: &'a OpenAiFederation,
    ) -> Self {
        Self {
            runners,
            federation: Some(federation),
        }
    }

    /// Returns the runner registry.
    pub fn runners_ref(self) -> &'a OpenAiRunnerRegistry {
        self.runners
    }

    /// Returns the federation surface, if one was configured.
    pub fn federation_ref(self) -> Option<&'a OpenAiFederation> {
        self.federation
    }
}
