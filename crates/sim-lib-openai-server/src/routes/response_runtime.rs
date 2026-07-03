use sim_kernel::ContentId;

use crate::{
    ids::GatewayIdGenerator,
    objects::{GatewayEvent, GatewayResponse},
    runtime::{OpenAiFederation, OpenAiRunnerRegistry},
};

use super::errors::OpenAiRouteError;

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

/// Holds the per-kind id generators used to mint request, run, response, and
/// event ids for a single response execution.
#[derive(Clone, Debug)]
pub struct ResponseIdGenerators {
    pub(crate) request: GatewayIdGenerator,
    pub(crate) run: GatewayIdGenerator,
    pub(crate) response: GatewayIdGenerator,
    event: GatewayIdGenerator,
}

impl ResponseIdGenerators {
    /// Builds id generators seeded deterministically from `start`, so a given
    /// seed always yields the same id sequence.
    pub fn deterministic(start: u64) -> Self {
        Self {
            request: GatewayIdGenerator::deterministic("gwreq", start),
            run: GatewayIdGenerator::deterministic("gwrun", start),
            response: GatewayIdGenerator::deterministic("resp", start),
            event: GatewayIdGenerator::deterministic("gwevt", start),
        }
    }

    pub(crate) fn next_event_id(&mut self) -> sim_kernel::Result<String> {
        self.event.next_id()
    }
}

/// Captures the outcome of executing a `/v1/responses` request: the wire
/// response plus the content-addressed ledger ids and events it produced.
#[derive(Clone, Debug)]
pub struct ResponseExecution {
    pub(crate) response: GatewayResponse,
    pub(crate) request_content_id: Option<ContentId>,
    pub(crate) run_content_id: Option<ContentId>,
    pub(crate) event_content_ids: Vec<ContentId>,
    pub(crate) events: Vec<GatewayEvent>,
    pub(crate) response_id: Option<String>,
    pub(crate) response_created_at_ms: Option<u64>,
    pub(crate) response_content_id: Option<ContentId>,
}

impl ResponseExecution {
    /// Returns the wire response produced by the execution.
    pub fn response(&self) -> &GatewayResponse {
        &self.response
    }

    /// Returns the content id of the stored request, if it was recorded.
    pub fn request_content_id(&self) -> Option<&ContentId> {
        self.request_content_id.as_ref()
    }

    /// Returns the content id of the stored run, if it was recorded.
    pub fn run_content_id(&self) -> Option<&ContentId> {
        self.run_content_id.as_ref()
    }

    /// Returns the content ids of the stored events, in sequence order.
    pub fn event_content_ids(&self) -> &[ContentId] {
        &self.event_content_ids
    }

    /// Returns the events emitted during the execution.
    pub fn events(&self) -> &[GatewayEvent] {
        &self.events
    }

    /// Returns the generated response id, if a response was produced.
    pub fn response_id(&self) -> Option<&str> {
        self.response_id.as_deref()
    }

    /// Returns the response creation timestamp in milliseconds, if set.
    pub fn response_created_at_ms(&self) -> Option<u64> {
        self.response_created_at_ms
    }

    /// Returns the content id of the stored response object, if it was recorded.
    pub fn response_content_id(&self) -> Option<&ContentId> {
        self.response_content_id.as_ref()
    }

    pub(crate) fn error(error: OpenAiRouteError) -> Self {
        Self {
            response: error.into_response(),
            request_content_id: None,
            run_content_id: None,
            event_content_ids: Vec::new(),
            events: Vec::new(),
            response_id: None,
            response_created_at_ms: None,
            response_content_id: None,
        }
    }
}
