use sim_kernel::CapabilityName;

/// Capability id `openai-gateway.serve` for serving the OpenAI gateway.
pub const OPENAI_GATEWAY_SERVE_CAPABILITY: &str = "openai-gateway.serve";
/// Capability id `openai-gateway.admin` for gateway administration.
pub const OPENAI_GATEWAY_ADMIN_CAPABILITY: &str = "openai-gateway.admin";
/// Capability id `openai-gateway.plan` for local plan execution.
pub const OPENAI_GATEWAY_PLAN_CAPABILITY: &str = "openai-gateway.plan";
/// Capability id `openai-gateway.plan.remote` for remote plan execution.
pub const OPENAI_GATEWAY_PLAN_REMOTE_CAPABILITY: &str = "openai-gateway.plan.remote";
/// Capability id `openai-gateway.federate` for federating to peer gateways.
pub const OPENAI_GATEWAY_FEDERATE_CAPABILITY: &str = "openai-gateway.federate";
/// Capability id `openai-gateway.tools` for invoking gateway tools.
pub const OPENAI_GATEWAY_TOOLS_CAPABILITY: &str = "openai-gateway.tools";
/// Capability id `ai-runner-cache` for using the AI runner plan cache.
pub const AI_RUNNER_CACHE_CAPABILITY: &str = "ai-runner-cache";
/// Capability id `network` for outbound network access.
pub const NETWORK_CAPABILITY: &str = "network";
/// Capability id `webhook-serve` for serving inbound webhook requests.
pub const WEBHOOK_SERVE_CAPABILITY: &str = "webhook-serve";

/// Returns the [`OPENAI_GATEWAY_SERVE_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_serve_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_SERVE_CAPABILITY)
}

/// Returns the [`OPENAI_GATEWAY_ADMIN_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_admin_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_ADMIN_CAPABILITY)
}

/// Returns the [`OPENAI_GATEWAY_PLAN_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_plan_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_PLAN_CAPABILITY)
}

/// Returns the [`OPENAI_GATEWAY_PLAN_REMOTE_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_plan_remote_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_PLAN_REMOTE_CAPABILITY)
}

/// Returns the [`OPENAI_GATEWAY_FEDERATE_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_federate_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_FEDERATE_CAPABILITY)
}

/// Returns the [`OPENAI_GATEWAY_TOOLS_CAPABILITY`] as a [`CapabilityName`].
pub fn openai_gateway_tools_capability() -> CapabilityName {
    CapabilityName::new(OPENAI_GATEWAY_TOOLS_CAPABILITY)
}

/// Returns the [`AI_RUNNER_CACHE_CAPABILITY`] as a [`CapabilityName`].
pub fn ai_runner_cache_capability() -> CapabilityName {
    CapabilityName::new(AI_RUNNER_CACHE_CAPABILITY)
}

/// Returns the [`NETWORK_CAPABILITY`] as a [`CapabilityName`].
pub fn network_capability() -> CapabilityName {
    CapabilityName::new(NETWORK_CAPABILITY)
}

/// Returns the [`WEBHOOK_SERVE_CAPABILITY`] as a [`CapabilityName`].
pub fn webhook_serve_capability() -> CapabilityName {
    CapabilityName::new(WEBHOOK_SERVE_CAPABILITY)
}

/// Returns the capability set required to serve the OpenAI gateway: serve,
/// network, and webhook-serve.
pub fn openai_gateway_serve_capabilities() -> Vec<CapabilityName> {
    vec![
        openai_gateway_serve_capability(),
        network_capability(),
        webhook_serve_capability(),
    ]
}
