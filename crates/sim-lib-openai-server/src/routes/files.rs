use serde_json::{Map, Value, json};
use sim_kernel::Expr;

use crate::{
    clock::{SystemWallClock, WallClock},
    content_id::content_id_for_expr,
    ids::GatewayIdGenerator,
    objects::{GatewayRequest, GatewayResponse, content_id_hex},
    server::GatewayRouteState,
    storage::{GatewayFile, GatewayFileStorageRef, GatewayStateStore},
};

use super::errors::OpenAiRouteError;

/// Route path for JSON file fixture upload (`POST /v1/files`).
pub const FILES_PATH: &str = "/v1/files";
/// Path prefix preceding a file id on the retrieval route (`/v1/files/`).
pub const FILE_RETRIEVAL_PREFIX: &str = "/v1/files/";
/// Templated route for retrieving a single file by id (`/v1/files/{id}`).
pub const FILE_RETRIEVAL_ROUTE: &str = "/v1/files/{id}";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Handles `POST /v1/files`, storing JSON `content` bytes and returning a file object.
pub fn handle_files(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemWallClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = GatewayIdGenerator::deterministic("file", seed);
    match state.store().lock() {
        Ok(mut store) => create_file(&mut *store, &mut ids, &mut clock, request)
            .unwrap_or_else(OpenAiRouteError::into_response),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `GET /v1/files/{id}`, returning the stored file's metadata object.
pub fn handle_file_retrieval(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let Some(file_id) = file_id_from_path(request.path()) else {
        return OpenAiRouteError::not_found_kind("file", request.path()).into_response();
    };
    match state.store().lock() {
        Ok(store) => retrieve_file(&*store, file_id),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Returns the JSON object for a stored file, or a not-found error response.
pub fn retrieve_file<S>(store: &S, file_id: &str) -> GatewayResponse
where
    S: GatewayStateStore,
{
    store
        .file(file_id)
        .map(|file| GatewayResponse::json_value(200, file_json(&file)))
        .unwrap_or_else(|| OpenAiRouteError::not_found_kind("file", file_id).into_response())
}

fn create_file<S, C>(
    store: &mut S,
    ids: &mut GatewayIdGenerator,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<GatewayResponse>
where
    S: GatewayStateStore,
    C: WallClock,
{
    let object = request_object(request.body())?;
    let filename = required_string(&object, "filename")?.to_owned();
    let purpose = object
        .get("purpose")
        .and_then(Value::as_str)
        .unwrap_or("assistants")
        .to_owned();
    let bytes = upload_bytes(&object)?;
    let content_id =
        content_id_for_expr(&Expr::Bytes(bytes.clone())).map_err(OpenAiRouteError::internal)?;
    let storage_ref = storage_ref(&object, content_id.clone())?;
    let file = GatewayFile::new(
        ids.next_id().map_err(OpenAiRouteError::internal)?,
        filename,
        purpose,
        bytes.len() as u64,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
        storage_ref,
    );
    store
        .put_file(file.clone(), bytes)
        .map_err(OpenAiRouteError::internal)?;
    Ok(GatewayResponse::json_value(200, file_json(&file)))
}

use crate::routes::request_json::{request_object, required_string};

fn upload_bytes(object: &Map<String, Value>) -> RouteResult<Vec<u8>> {
    object
        .get("content")
        .and_then(Value::as_str)
        .map(|content| content.as_bytes().to_vec())
        .ok_or_else(|| OpenAiRouteError::missing_required("content"))
}

fn storage_ref(
    object: &Map<String, Value>,
    content_id: sim_kernel::ContentId,
) -> RouteResult<GatewayFileStorageRef> {
    let Some(value) = object.get("storage_ref") else {
        return Ok(GatewayFileStorageRef::memory(content_id));
    };
    let storage = value.as_object().ok_or_else(|| {
        OpenAiRouteError::bad_request(
            "storage_ref must be an object",
            Some("storage_ref"),
            "invalid_storage_ref",
        )
    })?;
    match storage
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("memory")
    {
        "memory" => Ok(GatewayFileStorageRef::memory(content_id)),
        "table-fs" => storage
            .get("path")
            .and_then(Value::as_str)
            .map(GatewayFileStorageRef::table_fs)
            .ok_or_else(|| OpenAiRouteError::missing_required("path")),
        other => Err(OpenAiRouteError::bad_request(
            format!("unsupported file storage_ref kind: {other}"),
            Some("storage_ref"),
            "unsupported_storage_ref",
        )),
    }
}

fn file_id_from_path(path: &str) -> Option<&str> {
    super::path::id_from_path(path, FILE_RETRIEVAL_PREFIX)
}

fn file_json(file: &GatewayFile) -> Value {
    json!({
        "id": file.id(),
        "object": "file",
        "bytes": file.bytes(),
        "created_at": file.created_at_ms(),
        "filename": file.filename(),
        "purpose": file.purpose(),
        "storage_ref": storage_ref_json(file.storage_ref()),
    })
}

fn storage_ref_json(storage_ref: &GatewayFileStorageRef) -> Value {
    match storage_ref {
        GatewayFileStorageRef::Memory { content_id } => json!({
            "kind": "memory",
            "content_id": content_id_hex(content_id),
        }),
        GatewayFileStorageRef::TableFs { path } => json!({
            "kind": "table-fs",
            "path": path,
        }),
    }
}
