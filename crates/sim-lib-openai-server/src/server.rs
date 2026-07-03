use std::sync::{Arc, Mutex};

use sim_citizen_derive::non_citizen;
use sim_kernel::{Cx, Expr, Object, ObjectCompat, Ref, Result, Symbol};

pub use crate::objects::{GatewayRequest, GatewayResponse, GatewayResponseValue};
use crate::routes::{
    admin::{
        ADMIN_CACHE_STATS_PATH, ADMIN_CAPABILITY_REPORT_PATH, ADMIN_EVENTS_PATH,
        ADMIN_MODEL_HEALTH_PATH, ADMIN_RUN_RETRIEVAL_PREFIX, ADMIN_RUN_RETRIEVAL_ROUTE,
        ADMIN_RUNS_PATH, ADMIN_STORAGE_STATS_PATH, handle_admin_cache_stats,
        handle_admin_capability_report, handle_admin_events, handle_admin_model_health,
        handle_admin_run_get, handle_admin_runs, handle_admin_storage_stats,
    },
    audio::{
        AUDIO_SPEECH_PATH, AUDIO_TRANSCRIPTIONS_PATH, handle_audio_speech,
        handle_audio_transcriptions,
    },
    batches::{
        BATCH_CANCEL_ROUTE, BATCH_RETRIEVAL_PREFIX, BATCH_RETRIEVAL_ROUTE, BATCHES_PATH,
        handle_batch_cancel, handle_batch_retrieval, handle_batches,
    },
    chat_completions::{CHAT_COMPLETIONS_PATH, handle_chat_completions},
    embeddings::{EMBEDDINGS_PATH, handle_embeddings},
    files::{
        FILE_RETRIEVAL_PREFIX, FILE_RETRIEVAL_ROUTE, FILES_PATH, handle_file_retrieval,
        handle_files,
    },
    health::{HEALTH_PATH, handle_health},
    images::{IMAGES_GENERATIONS_PATH, handle_image_generations},
    models::{MODELS_PATH, handle_models},
    replay::{
        RESPONSE_EVENTS_ROUTE, RESPONSE_EVENTS_SUFFIX, RESPONSE_SIM_ROUTE, RESPONSE_SIM_SUFFIX,
        SIM_FORK_PATH, SIM_REPLAY_PATH, handle_response_events, handle_response_sim,
        handle_sim_fork, handle_sim_replay,
    },
    responses::{
        RESPONSE_RETRIEVAL_PREFIX, RESPONSE_RETRIEVAL_ROUTE, RESPONSES_PATH,
        handle_response_retrieval, handle_responses,
    },
    threads::{
        THREAD_MESSAGES_ROUTE, THREAD_RETRIEVAL_PREFIX, THREAD_RETRIEVAL_ROUTE, THREADS_PATH,
        handle_thread_get, handle_thread_post, handle_threads,
    },
    vector_stores::{
        VECTOR_STORE_SEARCH_PREFIX, VECTOR_STORE_SEARCH_ROUTE, VECTOR_STORE_SEARCH_SUFFIX,
        VECTOR_STORES_PATH, handle_vector_store_search, handle_vector_stores,
    },
};
use crate::{
    runtime::{
        OpenAiFederation, OpenAiKeyTable, OpenAiPlanCache, OpenAiRunnerRegistry,
        global_openai_key_table,
    },
    storage::MemoryGatewayStore,
};

type RouteHandler = fn(&GatewayRequest, &GatewayRouteState) -> GatewayResponse;

/// A single registered route: an HTTP method, a path matcher, and a handler.
#[derive(Clone, Copy)]
pub struct GatewayRoute {
    method: &'static str,
    path: RoutePath,
    handler: RouteHandler,
}

impl GatewayRoute {
    /// Returns a route matching `path` exactly for the given method and handler.
    pub fn new(method: &'static str, path: &'static str, handler: RouteHandler) -> Self {
        Self {
            method,
            path: RoutePath::Exact(path),
            handler,
        }
    }

    /// Returns a route matching any path that starts with `prefix`.
    ///
    /// `display_path` is the canonical path reported by [`GatewayRoute::path`].
    pub fn prefix(
        method: &'static str,
        display_path: &'static str,
        prefix: &'static str,
        handler: RouteHandler,
    ) -> Self {
        Self {
            method,
            path: RoutePath::Prefix {
                display_path,
                prefix,
            },
            handler,
        }
    }

    /// Returns a route matching paths that start with `prefix` and end with `suffix`.
    ///
    /// `display_path` is the canonical path reported by [`GatewayRoute::path`].
    pub fn prefix_suffix(
        method: &'static str,
        display_path: &'static str,
        prefix: &'static str,
        suffix: &'static str,
        handler: RouteHandler,
    ) -> Self {
        Self {
            method,
            path: RoutePath::PrefixSuffix {
                display_path,
                prefix,
                suffix,
            },
            handler,
        }
    }

    /// Returns the canonical display path for this route.
    pub fn path(&self) -> &'static str {
        self.path.display()
    }

    fn matches_path(&self, path: &str) -> bool {
        self.path.matches(path)
    }
}

#[derive(Clone, Copy)]
enum RoutePath {
    Exact(&'static str),
    Prefix {
        display_path: &'static str,
        prefix: &'static str,
    },
    PrefixSuffix {
        display_path: &'static str,
        prefix: &'static str,
        suffix: &'static str,
    },
}

impl RoutePath {
    fn display(self) -> &'static str {
        match self {
            Self::Exact(path) => path,
            Self::Prefix { display_path, .. } => display_path,
            Self::PrefixSuffix { display_path, .. } => display_path,
        }
    }

    fn matches(self, path: &str) -> bool {
        match self {
            Self::Exact(route_path) => route_path == path,
            Self::Prefix { prefix, .. } => path.starts_with(prefix),
            Self::PrefixSuffix { prefix, suffix, .. } => {
                path.starts_with(prefix) && path.ends_with(suffix)
            }
        }
    }
}

/// Shared, cloneable state threaded into every route handler.
///
/// Bundles the run store, plan cache, model runner registry, federation
/// registry, and API key table; clones share the same underlying store and
/// cache through interior mutability.
#[derive(Clone)]
pub struct GatewayRouteState {
    store: Arc<Mutex<MemoryGatewayStore>>,
    cache: Arc<Mutex<OpenAiPlanCache>>,
    runners: OpenAiRunnerRegistry,
    federation: OpenAiFederation,
    keys: OpenAiKeyTable,
}

impl GatewayRouteState {
    /// Returns route state wrapping `store` with a fresh cache, runners, and federation.
    pub fn new(store: MemoryGatewayStore) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            cache: Arc::new(Mutex::new(OpenAiPlanCache::new())),
            runners: OpenAiRunnerRegistry::new(),
            federation: OpenAiFederation::new(),
            keys: global_openai_key_table().clone(),
        }
    }

    /// Returns route state wrapping `store` and a preseeded plan `cache`.
    pub fn with_cache(store: MemoryGatewayStore, cache: OpenAiPlanCache) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            cache: Arc::new(Mutex::new(cache)),
            runners: OpenAiRunnerRegistry::new(),
            federation: OpenAiFederation::new(),
            keys: global_openai_key_table().clone(),
        }
    }

    /// Returns the state with its model runner registry replaced by `runners`.
    pub fn with_runners(mut self, runners: OpenAiRunnerRegistry) -> Self {
        self.runners = runners;
        self
    }

    /// Returns the state with its federation registry replaced by `federation`.
    pub fn with_federation(mut self, federation: OpenAiFederation) -> Self {
        self.federation = federation;
        self
    }

    /// Returns the state with its API key table replaced by `keys`.
    pub fn with_keys(mut self, keys: OpenAiKeyTable) -> Self {
        self.keys = keys;
        self
    }

    /// Returns route state backed by a fresh in-memory store.
    pub fn memory() -> Self {
        Self::new(MemoryGatewayStore::new())
    }

    /// Returns the shared run store.
    pub fn store(&self) -> &Arc<Mutex<MemoryGatewayStore>> {
        &self.store
    }

    /// Returns the shared plan cache.
    pub fn cache(&self) -> &Arc<Mutex<OpenAiPlanCache>> {
        &self.cache
    }

    /// Returns the model runner registry.
    pub fn runners(&self) -> &OpenAiRunnerRegistry {
        &self.runners
    }

    /// Returns the federation registry.
    pub fn federation(&self) -> &OpenAiFederation {
        &self.federation
    }

    /// Returns the API key table.
    pub fn keys(&self) -> &OpenAiKeyTable {
        &self.keys
    }
}

/// A route table paired with the state handlers dispatch against.
#[derive(Clone)]
pub struct GatewayRoutes {
    routes: Vec<GatewayRoute>,
    state: GatewayRouteState,
}

impl GatewayRoutes {
    /// Returns a route table over `routes` with fresh in-memory state.
    pub fn new(routes: Vec<GatewayRoute>) -> Self {
        Self::with_state(routes, GatewayRouteState::memory())
    }

    /// Returns a route table over `routes` sharing the supplied `state`.
    pub fn with_state(routes: Vec<GatewayRoute>, state: GatewayRouteState) -> Self {
        Self { routes, state }
    }

    /// Dispatches `request` to the matching route, returning its response.
    ///
    /// Returns 405 when the path exists under a different method and 404 when no
    /// route matches the path at all.
    pub fn handle(&self, request: &GatewayRequest) -> GatewayResponse {
        let mut path_exists = false;
        for route in &self.routes {
            if !route.matches_path(request.path()) {
                continue;
            }
            path_exists = true;
            if route.method == request.method() {
                return (route.handler)(request, &self.state);
            }
        }
        if path_exists {
            GatewayResponse::text(405, "method not allowed")
        } else {
            GatewayResponse::text(404, "not found")
        }
    }

    /// Returns the display paths of all registered routes.
    pub fn paths(&self) -> Vec<&'static str> {
        self.routes.iter().map(GatewayRoute::path).collect()
    }

    /// Returns the shared route state.
    pub fn state(&self) -> &GatewayRouteState {
        &self.state
    }
}

/// Returns the full gateway route table backed by fresh in-memory state.
pub fn configure_routes() -> GatewayRoutes {
    configure_routes_with_state(GatewayRouteState::memory())
}

/// Returns the full gateway route table sharing the supplied `state`.
///
/// Registers every OpenAI-compatible endpoint (chat, responses, embeddings,
/// files, batches, audio, images, vector stores, threads) plus the SIM replay
/// and admin routes.
pub fn configure_routes_with_state(state: GatewayRouteState) -> GatewayRoutes {
    GatewayRoutes::with_state(
        vec![
            GatewayRoute::new("GET", HEALTH_PATH, handle_health),
            GatewayRoute::new("GET", MODELS_PATH, handle_models),
            GatewayRoute::new("POST", RESPONSES_PATH, handle_responses),
            GatewayRoute::new("POST", SIM_REPLAY_PATH, handle_sim_replay),
            GatewayRoute::new("POST", SIM_FORK_PATH, handle_sim_fork),
            GatewayRoute::new("POST", CHAT_COMPLETIONS_PATH, handle_chat_completions),
            GatewayRoute::new("POST", EMBEDDINGS_PATH, handle_embeddings),
            GatewayRoute::new("POST", FILES_PATH, handle_files),
            GatewayRoute::new("POST", BATCHES_PATH, handle_batches),
            GatewayRoute::new(
                "POST",
                AUDIO_TRANSCRIPTIONS_PATH,
                handle_audio_transcriptions,
            ),
            GatewayRoute::new("POST", AUDIO_SPEECH_PATH, handle_audio_speech),
            GatewayRoute::new("POST", IMAGES_GENERATIONS_PATH, handle_image_generations),
            GatewayRoute::new("POST", VECTOR_STORES_PATH, handle_vector_stores),
            GatewayRoute::prefix_suffix(
                "POST",
                VECTOR_STORE_SEARCH_ROUTE,
                VECTOR_STORE_SEARCH_PREFIX,
                VECTOR_STORE_SEARCH_SUFFIX,
                handle_vector_store_search,
            ),
            GatewayRoute::prefix(
                "GET",
                BATCH_RETRIEVAL_ROUTE,
                BATCH_RETRIEVAL_PREFIX,
                handle_batch_retrieval,
            ),
            GatewayRoute::prefix(
                "POST",
                BATCH_CANCEL_ROUTE,
                BATCH_RETRIEVAL_PREFIX,
                handle_batch_cancel,
            ),
            GatewayRoute::prefix(
                "GET",
                FILE_RETRIEVAL_ROUTE,
                FILE_RETRIEVAL_PREFIX,
                handle_file_retrieval,
            ),
            GatewayRoute::new("POST", THREADS_PATH, handle_threads),
            GatewayRoute::prefix(
                "POST",
                THREAD_MESSAGES_ROUTE,
                THREAD_RETRIEVAL_PREFIX,
                handle_thread_post,
            ),
            GatewayRoute::prefix(
                "GET",
                THREAD_RETRIEVAL_ROUTE,
                THREAD_RETRIEVAL_PREFIX,
                handle_thread_get,
            ),
            GatewayRoute::prefix_suffix(
                "GET",
                RESPONSE_EVENTS_ROUTE,
                RESPONSE_RETRIEVAL_PREFIX,
                RESPONSE_EVENTS_SUFFIX,
                handle_response_events,
            ),
            GatewayRoute::prefix_suffix(
                "GET",
                RESPONSE_SIM_ROUTE,
                RESPONSE_RETRIEVAL_PREFIX,
                RESPONSE_SIM_SUFFIX,
                handle_response_sim,
            ),
            GatewayRoute::prefix(
                "GET",
                RESPONSE_RETRIEVAL_ROUTE,
                RESPONSE_RETRIEVAL_PREFIX,
                handle_response_retrieval,
            ),
            GatewayRoute::new("GET", ADMIN_RUNS_PATH, handle_admin_runs),
            GatewayRoute::prefix(
                "GET",
                ADMIN_RUN_RETRIEVAL_ROUTE,
                ADMIN_RUN_RETRIEVAL_PREFIX,
                handle_admin_run_get,
            ),
            GatewayRoute::new("GET", ADMIN_EVENTS_PATH, handle_admin_events),
            GatewayRoute::new("GET", ADMIN_STORAGE_STATS_PATH, handle_admin_storage_stats),
            GatewayRoute::new("GET", ADMIN_MODEL_HEALTH_PATH, handle_admin_model_health),
            GatewayRoute::new("GET", ADMIN_CACHE_STATS_PATH, handle_admin_cache_stats),
            GatewayRoute::new(
                "GET",
                ADMIN_CAPABILITY_REPORT_PATH,
                handle_admin_capability_report,
            ),
        ],
        state,
    )
}

/// A [`GatewayRoutes`] table wrapped as a runtime [`Object`].
#[derive(Clone)]
#[non_citizen(
    reason = "live OpenAI gateway route table handle; route requests use openai/GatewayRequest descriptor",
    kind = "handle"
)]
pub struct GatewayRoutesValue {
    routes: GatewayRoutes,
}

impl GatewayRoutesValue {
    /// Wraps a route table as a runtime object value.
    pub fn new(routes: GatewayRoutes) -> Self {
        Self { routes }
    }

    /// Returns the wrapped route table.
    pub fn routes(&self) -> &GatewayRoutes {
        &self.routes
    }
}

impl Object for GatewayRoutesValue {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<openai-gateway-routes>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for GatewayRoutesValue {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Map(vec![(
            Expr::Symbol(Symbol::new("routes")),
            Expr::List(
                self.routes
                    .paths()
                    .into_iter()
                    .map(|path| Expr::String(path.to_owned()))
                    .collect(),
            ),
        )]))
    }
}

/// Returns the [`Ref`] naming the gateway health endpoint (`openai-gateway/health`).
pub fn health_ref() -> Ref {
    Ref::Symbol(Symbol::qualified("openai-gateway", "health"))
}
