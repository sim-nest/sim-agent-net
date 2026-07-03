use sim_kernel::{ContentId, Result};

use crate::objects::{GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun};

/// Content-addressed store for the gateway's eval-pipeline records.
///
/// Each record is keyed by its [`ContentId`], so writing the same value twice
/// is idempotent. Concrete backends implement this trait over in-memory maps
/// or SIM tables.
pub trait GatewayStore {
    /// Stores a request under its content id.
    fn put_request(&mut self, id: ContentId, request: GatewayRequest) -> Result<()>;
    /// Returns the request stored under `id`, if present.
    fn request(&self, id: &ContentId) -> Option<GatewayRequest>;

    /// Stores a run under its content id.
    fn put_run(&mut self, id: ContentId, run: GatewayRun) -> Result<()>;
    /// Returns the run stored under `id`, if present.
    fn run(&self, id: &ContentId) -> Option<GatewayRun>;

    /// Stores an event under its content id.
    fn put_event(&mut self, id: ContentId, event: GatewayEvent) -> Result<()>;
    /// Returns the event stored under `id`, if present.
    fn event(&self, id: &ContentId) -> Option<GatewayEvent>;

    /// Stores a response under its content id.
    fn put_response(&mut self, id: ContentId, response: GatewayResponse) -> Result<()>;
    /// Returns the response stored under `id`, if present.
    fn response(&self, id: &ContentId) -> Option<GatewayResponse>;
}
