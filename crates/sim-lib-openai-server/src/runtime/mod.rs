/// Plan evaluation cache: keys, modes, write targets, and the in-memory store.
pub mod cache;
/// The OpenAI gateway eval fabric implementing the `EvalFabric` surface.
pub mod fabric;
/// Federated inference: remote gateway registry and request/response policy.
pub mod federation;
/// API key tables, secret hashing, and per-request capability resolution.
pub mod keys;
/// Registry mapping OpenAI model ids to local model runners.
pub mod runners;
/// Multi-round tool-call loop driving model runs that invoke tools.
pub mod tool_loop;

pub use cache::{OpenAiPlanCache, PlanCacheKey, PlanCacheMode, PlanCacheWriteTarget};
pub use fabric::OpenAiGatewayFabric;
pub use federation::{OpenAiFederatedGateway, OpenAiFederation, OpenAiFederationPolicy};
pub use keys::{
    OPENAI_GATEWAY_KEY_OBJECT, OpenAiGatewayKey, OpenAiKeyTable, global_openai_key_table,
    grant_capability_set, key_hash, redacted_gateway_request,
};
pub use runners::OpenAiRunnerRegistry;
pub use tool_loop::{ToolLoopConfig, run_tool_loop_with_cache, run_tool_loop_with_registry};
