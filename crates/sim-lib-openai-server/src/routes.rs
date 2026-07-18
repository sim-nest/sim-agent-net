/// Admin inspection routes (runs, events, storage, health, cache, capabilities).
pub mod admin;
/// OpenAI-shaped audio fixture handlers (transcription and speech surfaces).
pub mod audio;
/// OpenAI batch endpoint handlers.
pub mod batches;
/// OpenAI `/v1/chat/completions` endpoint handlers.
pub mod chat_completions;
/// OpenAI `/v1/embeddings` endpoint handlers.
pub mod embeddings;
/// Shared route error type and OpenAI-shaped error responses.
pub mod errors;
/// Shared gateway execution-record substrate (id generators, event log,
/// request+run prologue, and the run outcome struct).
pub(crate) mod execution_record;
/// OpenAI-shaped JSON file fixture handlers.
pub mod files;
/// Health and readiness route handlers.
pub mod health;
/// OpenAI-shaped image fixture handlers.
pub mod images;
/// OpenAI `/v1/models` discovery endpoint and model catalog.
pub mod models;
/// Shared path-id extraction (`strip_prefix` + single-segment filter).
pub(crate) mod path;
/// SIM replay and fork routes over stored responses.
pub mod replay;
/// Helpers for parsing JSON request bodies into objects.
pub(crate) mod request_json;
/// Shared response-execution runtime types (targets, id generators, outcome).
pub mod response_runtime;
/// Helpers for chunking model output into streamed response text deltas.
pub(crate) mod response_text;
/// OpenAI `/v1/responses` endpoint handlers and request execution.
pub mod responses;
/// Internal run-record helpers.
pub(crate) mod run_record;
/// Thread-context normalization for response requests.
pub(crate) mod thread_context;
/// OpenAI threads endpoint handlers.
pub mod threads;
/// OpenAI-shaped vector-store subset handlers.
pub mod vector_stores;
