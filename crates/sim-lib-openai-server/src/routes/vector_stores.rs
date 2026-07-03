use serde_json::{Map, Value, json};
use sim_kernel::Expr;

use crate::{
    clock::{GatewayClock, SystemGatewayClock},
    ids::GatewayIdGenerator,
    objects::{GatewayRequest, GatewayResponse},
    routes::{
        embeddings::TENSOR_F64_SMALL_EMBEDDING_MODEL,
        request_json::{
            optional_string, optional_u64, record_execution, request_object, required_string,
        },
        run_record::{
            RouteEventInput, RouteRunExecution, RouteRunIdGenerators, RouteRunRecord, field,
            record_route_execution,
        },
    },
    server::GatewayRouteState,
    storage::{GatewayStateStore, GatewayStore, GatewayVectorStore, GatewayVectorStoreItem},
};

use super::errors::OpenAiRouteError;

/// Route path for vector store creation (`POST /v1/vector_stores`).
pub const VECTOR_STORES_PATH: &str = "/v1/vector_stores";
/// Path prefix preceding a vector store id on the search route (`/v1/vector_stores/`).
pub const VECTOR_STORE_SEARCH_PREFIX: &str = "/v1/vector_stores/";
/// Path suffix that marks a vector store search route (`/search`).
pub const VECTOR_STORE_SEARCH_SUFFIX: &str = "/search";
/// Templated route for searching a vector store (`/v1/vector_stores/{id}/search`).
pub const VECTOR_STORE_SEARCH_ROUTE: &str = "/v1/vector_stores/{id}/search";

const VECTOR_DIMENSION: usize = 8;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const EMBEDDING_SCALE: u64 = 1_000_000;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

#[derive(Clone, Debug)]
pub(crate) struct VectorStoreIdGenerators {
    vector_store: GatewayIdGenerator,
    item: GatewayIdGenerator,
    run: RouteRunIdGenerators,
}

impl VectorStoreIdGenerators {
    pub(crate) fn deterministic(start: u64) -> Self {
        Self {
            vector_store: GatewayIdGenerator::deterministic("vs", start),
            item: GatewayIdGenerator::deterministic("vsi", start),
            run: RouteRunIdGenerators::deterministic(start),
        }
    }
}

/// Handles `POST /v1/vector_stores`, creating a vector store with deterministic
/// embeddings for its items and returning the store object.
pub fn handle_vector_stores(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = VectorStoreIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => {
            execute_vector_store_create_request(&mut *store, &mut ids, &mut clock, request)
                .response()
                .clone()
        }
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/vector_stores/{id}/search`, returning cosine-ranked search
/// results for the query against the stored items.
pub fn handle_vector_store_search(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let Some(vector_store_id) = vector_store_id_from_search_path(request.path()) else {
        return OpenAiRouteError::not_found_kind("vector_store", request.path()).into_response();
    };
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = RouteRunIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => execute_vector_store_search_request(
            &mut *store,
            &mut ids,
            &mut clock,
            request,
            vector_store_id,
        )
        .response()
        .clone(),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

pub(crate) fn execute_vector_store_create_request<S, C>(
    store: &mut S,
    ids: &mut VectorStoreIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteRunExecution
where
    S: GatewayStore + GatewayStateStore,
    C: GatewayClock,
{
    match try_execute_vector_store_create_request(store, ids, clock, request) {
        Ok(execution) => execution,
        Err(error) => RouteRunExecution::error(error),
    }
}

pub(crate) fn execute_vector_store_search_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    vector_store_id: &str,
) -> RouteRunExecution
where
    S: GatewayStore + GatewayStateStore,
    C: GatewayClock,
{
    match try_execute_vector_store_search_request(store, ids, clock, request, vector_store_id) {
        Ok(execution) => execution,
        Err(error) => RouteRunExecution::error(error),
    }
}

fn try_execute_vector_store_create_request<S, C>(
    store: &mut S,
    ids: &mut VectorStoreIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore + GatewayStateStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let name = optional_string(&object, "name", "sim-vector-store");
    let item_specs = item_specs(&object)?;
    let items = item_specs
        .into_iter()
        .map(|spec| {
            let id = spec
                .id
                .map(Ok)
                .unwrap_or_else(|| ids.item.next_id().map_err(OpenAiRouteError::internal))?;
            Ok(GatewayVectorStoreItem::new(
                id,
                spec.text.clone(),
                embedding_for_text(&spec.text),
            ))
        })
        .collect::<RouteResult<Vec<_>>>()?;
    let vector_store = GatewayVectorStore::new(
        ids.vector_store
            .next_id()
            .map_err(OpenAiRouteError::internal)?,
        name,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
        items,
    );
    store
        .put_vector_store(vector_store.clone())
        .map_err(OpenAiRouteError::internal)?;
    let response = GatewayResponse::json(
        200,
        vector_store_json(&vector_store).to_string().into_bytes(),
    );
    record_route_execution(
        store,
        &mut ids.run,
        clock,
        request,
        RouteRunRecord::new(
            response,
            VECTOR_STORES_PATH,
            vec![RouteEventInput::new(
                "vector-store-create",
                vector_store_event_payload(&vector_store),
            )],
            record_execution(&object),
        ),
    )
}

fn try_execute_vector_store_search_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    vector_store_id: &str,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore + GatewayStateStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let query = required_string(&object, "query")?;
    let limit = optional_u64(&object, "limit", optional_u64(&object, "top_k", 10)?)?;
    let vector_store = store
        .vector_store(vector_store_id)
        .ok_or_else(|| OpenAiRouteError::not_found_kind("vector_store", vector_store_id))?;
    let hits = search_vector_store(&vector_store, query, limit as usize);
    let response = GatewayResponse::json(
        200,
        search_json(vector_store_id, query, &hits)
            .to_string()
            .into_bytes(),
    );
    record_route_execution(
        store,
        ids,
        clock,
        request,
        RouteRunRecord::new(
            response,
            VECTOR_STORE_SEARCH_ROUTE,
            vec![RouteEventInput::new(
                "vector-store-search",
                search_event_payload(vector_store_id, query, hits.len()),
            )],
            record_execution(&object),
        ),
    )
}

#[derive(Clone, Debug)]
struct ItemSpec {
    id: Option<String>,
    text: String,
}

fn item_specs(object: &Map<String, Value>) -> RouteResult<Vec<ItemSpec>> {
    let Some(value) = object.get("items") else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(OpenAiRouteError::bad_request(
            "items must be an array",
            Some("items"),
            "invalid_request",
        ));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(ItemSpec {
                id: None,
                text: text.clone(),
            }),
            Value::Object(object) => object_item_spec(object),
            _ => Err(OpenAiRouteError::bad_request(
                "items must contain strings or objects",
                Some("items"),
                "invalid_request",
            )),
        })
        .collect()
}

fn object_item_spec(object: &Map<String, Value>) -> RouteResult<ItemSpec> {
    Ok(ItemSpec {
        id: object.get("id").and_then(Value::as_str).map(str::to_owned),
        text: required_string(object, "text")?.to_owned(),
    })
}

#[derive(Clone, Debug)]
struct SearchHit {
    id: String,
    text: String,
    score: f64,
}

fn search_vector_store(store: &GatewayVectorStore, query: &str, limit: usize) -> Vec<SearchHit> {
    let query_embedding = embedding_for_text(query);
    let mut hits = store
        .items()
        .iter()
        .map(|item| SearchHit {
            id: item.id().to_owned(),
            text: item.text().to_owned(),
            score: cosine_score(&query_embedding, item.embedding()),
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    hits.truncate(limit);
    hits
}

fn cosine_score(left: &[f64], right: &[f64]) -> f64 {
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f64>();
    let left_norm = left.iter().map(|value| value * value).sum::<f64>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f64>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn embedding_for_text(input: &str) -> Vec<f64> {
    (0..VECTOR_DIMENSION)
        .map(|index| hash_to_unit(stable_embedding_hash(input, index)))
        .collect()
}

fn stable_embedding_hash(input: &str, dimension_index: usize) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    mix_bytes(&mut hash, TENSOR_F64_SMALL_EMBEDDING_MODEL.as_bytes());
    mix_byte(&mut hash, 0xff);
    mix_bytes(&mut hash, input.as_bytes());
    mix_byte(&mut hash, 0xfe);
    mix_bytes(&mut hash, &dimension_index.to_le_bytes());
    hash
}

fn mix_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        mix_byte(hash, *byte);
    }
}

fn mix_byte(hash: &mut u64, byte: u8) {
    *hash ^= u64::from(byte);
    *hash = hash.wrapping_mul(FNV_PRIME);
}

fn hash_to_unit(hash: u64) -> f64 {
    let bucket = hash % (EMBEDDING_SCALE * 2 + 1);
    (bucket as f64 / EMBEDDING_SCALE as f64) - 1.0
}

fn vector_store_id_from_search_path(path: &str) -> Option<&str> {
    path.strip_prefix(VECTOR_STORE_SEARCH_PREFIX)
        .and_then(|rest| rest.strip_suffix(VECTOR_STORE_SEARCH_SUFFIX))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

fn vector_store_json(vector_store: &GatewayVectorStore) -> Value {
    let item_count = vector_store.items().len() as u64;
    json!({
        "id": vector_store.id(),
        "object": "vector_store",
        "created_at": vector_store.created_at_ms(),
        "name": vector_store.name(),
        "metadata": {},
        "file_counts": {
            "in_progress": 0,
            "completed": item_count,
            "failed": 0,
            "cancelled": 0,
            "total": item_count,
        },
    })
}

fn search_json(vector_store_id: &str, query: &str, hits: &[SearchHit]) -> Value {
    json!({
        "object": "vector_store.search_results",
        "vector_store_id": vector_store_id,
        "query": query,
        "model": TENSOR_F64_SMALL_EMBEDDING_MODEL,
        "data": hits
            .iter()
            .map(|hit| {
                json!({
                    "object": "vector_store.search_result",
                    "id": hit.id,
                    "text": hit.text,
                    "score": rounded_score(hit.score),
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn rounded_score(score: f64) -> f64 {
    (score * 1_000_000.0).round() / 1_000_000.0
}

fn vector_store_event_payload(vector_store: &GatewayVectorStore) -> Expr {
    Expr::Map(vec![
        field("id", Expr::String(vector_store.id().to_owned())),
        field("name", Expr::String(vector_store.name().to_owned())),
        field(
            "item-count",
            Expr::String(vector_store.items().len().to_string()),
        ),
        field(
            "embedding-model",
            Expr::String(TENSOR_F64_SMALL_EMBEDDING_MODEL.to_owned()),
        ),
    ])
}

fn search_event_payload(vector_store_id: &str, query: &str, result_count: usize) -> Expr {
    Expr::Map(vec![
        field("vector-store-id", Expr::String(vector_store_id.to_owned())),
        field("query", Expr::String(query.to_owned())),
        field("result-count", Expr::String(result_count.to_string())),
    ])
}
