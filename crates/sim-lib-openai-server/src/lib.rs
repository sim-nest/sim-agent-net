#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! OpenAI-shaped gateway routes for SIM.
//!
//! This crate provides the OpenAI gateway health route, OpenAI JSON transcript
//! codec, model discovery, gateway object helpers, plan parsing and fixture atom
//! execution through `POST /v1/responses` and `POST /v1/chat/completions`,
//! embeddings through `POST /v1/embeddings`, SSE streaming projection,
//! memory-backed response retrieval by id, and stored response replay/fork
//! inspection routes. File, audio, image, and vector-store routes are SIM JSON
//! fixture/subset surfaces: they use OpenAI-shaped paths and response objects,
//! while request bodies use the explicit JSON fields handled by these route
//! modules rather than multipart upload or provider media protocols. The model
//! list route advertises models from registered `ModelCard` records, so
//! loadable local placement sites appear beside fixture and remote runner
//! entries when installed.

/// Capability identifiers and constructors that gate gateway operations.
pub mod capabilities;
/// Citizen descriptor types bridging gateway objects into the SIM object graph.
pub mod citizen;
/// Compatibility names for the canonical server wall-clock contract.
pub mod clock;
/// OpenAI JSON codec surface: decode, encode, streaming, and shared shapes.
pub mod codec_openai;
/// Content-id helpers that normalize and hash gateway request expressions.
pub mod content_id;
/// Identifier generation for gateway requests, responses, and runs.
pub mod ids;
/// Installation glue that registers the gateway library with a runtime.
pub mod install;
/// Library manifest naming and export wiring for the gateway.
pub mod manifest;
/// Core gateway object records: requests, responses, runs, and events.
pub mod objects;
/// Gateway operations exposed to the runtime as callable functions.
pub mod ops;
mod ops_admin;
/// Plan parsing, checking, evaluation, and combinator surface.
pub mod plan;
/// HTTP route handlers mapping OpenAI endpoints onto SIM eval and agents.
pub mod routes;
/// Runtime support: key tables, plan cache, federation, runners, and fabric.
pub mod runtime;
/// Server wiring that registers gateway routes and holds route state.
pub mod server;
/// Persistent and in-memory stores for gateway records and content objects.
pub mod storage;
/// Translation between OpenAI tool/function shapes and SIM symbols.
pub mod translate;

pub use capabilities::{
    AI_RUNNER_CACHE_CAPABILITY, NETWORK_CAPABILITY, OPENAI_GATEWAY_ADMIN_CAPABILITY,
    OPENAI_GATEWAY_FEDERATE_CAPABILITY, OPENAI_GATEWAY_PLAN_CAPABILITY,
    OPENAI_GATEWAY_PLAN_REMOTE_CAPABILITY, OPENAI_GATEWAY_SERVE_CAPABILITY,
    OPENAI_GATEWAY_TOOLS_CAPABILITY, WEBHOOK_SERVE_CAPABILITY, ai_runner_cache_capability,
    network_capability, openai_gateway_admin_capability, openai_gateway_federate_capability,
    openai_gateway_plan_capability, openai_gateway_plan_remote_capability,
    openai_gateway_serve_capabilities, openai_gateway_serve_capability,
    openai_gateway_tools_capability, require_openai_gateway_serve_capabilities,
    webhook_serve_capability,
};
#[allow(deprecated)]
pub use clock::{DeterministicGatewayClock, GatewayClock, SystemGatewayClock};
pub use codec_openai::{
    ChatTranscript, GatewayEventData, OpenAiCodec, OpenAiCodecOptions, OpenAiRequestOptions,
    OpenAiSseSurface, StreamSink, decode_openai_request, decode_openai_response,
    encode_gateway_events_sse, encode_openai_request, encode_openai_response,
    encode_openai_responses_response, gateway_event_data_from_packet, gateway_event_data_kind,
    gateway_event_data_packets, openai_codec_symbol,
};
pub use content_id::{
    VOLATILE_REQUEST_FIELDS, content_id_for_expr, normalize_request_expr, request_content_id,
    request_expr_content_id,
};
pub use ids::GatewayIdGenerator;
pub use install::install_openai_gateway_lib;
pub use manifest::{OpenAiGatewayLib, manifest_name};
pub use objects::{
    GATEWAY_EVENT_OBJECT, GATEWAY_REQUEST_OBJECT, GATEWAY_RESPONSE_OBJECT, GATEWAY_RUN_OBJECT,
    GatewayEvent, GatewayRequest, GatewayResponse, GatewayResponseValue, GatewayRun,
    content_id_expr, content_id_hex,
};
pub use ops::{
    OpenAiGatewayFunction, cache_stats_symbol, capability_report_symbol, events_symbol,
    fabric_symbol, health_symbol, key_add_symbol, key_list_symbol, model_health_symbol,
    models_symbol, plan_check_symbol, plan_combinators_symbol, plan_explain_symbol,
    plan_parse_symbol, plan_run_symbol, run_get_symbol, runs_symbol, serve_symbol,
    storage_stats_symbol,
};
pub use plan::{
    BackendDescriptor, PlanCombinator, PlanEvalEvent, PlanEvalReport, PlanLimits, check_plan,
    check_plan_with_limits, eval_plan, eval_plan_report, eval_plan_report_with_cache,
    eval_plan_report_with_cache_and_runners, eval_plan_report_with_cache_runners_and_federation,
    eval_plan_report_with_federation, explain_plan, parse_plan, plan_combinators,
    plan_combinators_expr, plan_symbol, provider_prefixes, resolve_atom_address,
};
pub use routes::admin::{
    ADMIN_CACHE_STATS_PATH, ADMIN_CAPABILITY_REPORT_PATH, ADMIN_EVENTS_PATH,
    ADMIN_MODEL_HEALTH_PATH, ADMIN_RUN_RETRIEVAL_PREFIX, ADMIN_RUN_RETRIEVAL_ROUTE,
    ADMIN_RUNS_PATH, ADMIN_STORAGE_STATS_PATH,
};
pub use routes::audio::{
    AUDIO_SPEECH_PATH, AUDIO_TRANSCRIPTIONS_PATH, handle_audio_speech, handle_audio_transcriptions,
};
pub use routes::batches::{
    BATCH_CANCEL_ROUTE, BATCH_RETRIEVAL_PREFIX, BATCH_RETRIEVAL_ROUTE, BATCHES_PATH,
    handle_batch_cancel, handle_batch_retrieval, handle_batches, retrieve_batch,
};
pub use routes::chat_completions::{
    CHAT_COMPLETIONS_PATH, ChatCompletionExecution, execute_chat_completion_request,
    execute_chat_completion_request_with_runners,
    execute_chat_completion_request_with_runners_and_federation, handle_chat_completions,
};
pub use routes::embeddings::{
    EMBEDDINGS_PATH, EmbeddingExecution, EmbeddingIdGenerators, TENSOR_F64_SMALL_EMBEDDING_MODEL,
    execute_embedding_request, handle_embeddings,
};
pub use routes::files::{
    FILE_RETRIEVAL_PREFIX, FILE_RETRIEVAL_ROUTE, FILES_PATH, handle_file_retrieval, handle_files,
    retrieve_file,
};
pub use routes::health::{HEALTH_BODY, HEALTH_PATH, health_response};
pub use routes::images::{IMAGES_GENERATIONS_PATH, handle_image_generations};
pub use routes::models::{
    MODELS_PATH, ModelCatalog, OpenAiModel, models_response, models_response_for_catalog,
    models_response_for_runner_args,
};
pub use routes::replay::{
    RESPONSE_EVENTS_ROUTE, RESPONSE_EVENTS_SUFFIX, RESPONSE_SIM_ROUTE, RESPONSE_SIM_SUFFIX,
    SIM_EXTENSION_CAPABILITY, SIM_FORK_PATH, SIM_INSPECTION_CAPABILITY, SIM_REPLAY_PATH,
    handle_response_events, handle_response_sim, handle_sim_fork, handle_sim_replay,
    response_events, response_sim,
};
pub use routes::responses::{
    RESPONSE_RETRIEVAL_PREFIX, RESPONSE_RETRIEVAL_ROUTE, RESPONSES_PATH, ResponseExecution,
    ResponseIdGenerators, ResponseRuntimeTargets, execute_response_request,
    execute_response_request_with_cache, execute_response_request_with_cache_and_runners,
    execute_response_request_with_cache_runners_and_federation,
    execute_response_request_with_runners, handle_response_retrieval, handle_responses,
    retrieve_response,
};
pub use routes::threads::{
    THREAD_MESSAGES_ROUTE, THREAD_RETRIEVAL_PREFIX, THREAD_RETRIEVAL_ROUTE, THREADS_PATH,
    handle_thread_get, handle_thread_post, handle_threads, list_messages, retrieve_thread,
};
pub use routes::vector_stores::{
    VECTOR_STORE_SEARCH_PREFIX, VECTOR_STORE_SEARCH_ROUTE, VECTOR_STORE_SEARCH_SUFFIX,
    VECTOR_STORES_PATH, handle_vector_store_search, handle_vector_stores,
};
pub use runtime::{
    OPENAI_GATEWAY_KEY_OBJECT, OpenAiFederatedGateway, OpenAiFederation, OpenAiFederationPolicy,
    OpenAiGatewayFabric, OpenAiGatewayKey, OpenAiKeyTable, OpenAiPlanCache, OpenAiRunnerRegistry,
    PlanCacheKey, PlanCacheMode, PlanCacheWriteTarget, ToolLoopConfig, global_openai_key_table,
    key_hash, redacted_gateway_request, run_tool_loop_with_cache, run_tool_loop_with_registry,
};
pub use server::{
    GatewayRouteState, GatewayRoutes, GatewayRoutesValue, configure_routes,
    configure_routes_with_state,
};
pub use sim_lib_server::{DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp};
pub use storage::{
    GATEWAY_BATCH_KIND, GATEWAY_FILE_KIND, GATEWAY_THREAD_KIND, GATEWAY_THREAD_MESSAGE_KIND,
    GATEWAY_VECTOR_STORE_ITEM_KIND, GATEWAY_VECTOR_STORE_KIND, GatewayBatch, GatewayBatchCounts,
    GatewayBatchStatus, GatewayFile, GatewayFileStorageRef, GatewayResponseObjectStore,
    GatewayStateStore, GatewayStore, GatewayThread, GatewayThreadMessage, GatewayVectorStore,
    GatewayVectorStoreItem, MemoryGatewayStore, StoredGatewayResponse, TableGatewayStore,
};
pub use translate::tools::{
    OpenAiTool, OpenAiToolCall, OpenAiToolRegistry, OpenAiToolResult, openai_name_to_symbol,
    symbol_from_text,
};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
