use sim_kernel::{ContentId, Expr, Result, Symbol};

use crate::objects::{GatewayResponse, content_id_expr};

use super::vector::GatewayVectorStore;

/// Record kind tag designating a gateway file.
pub const GATEWAY_FILE_KIND: &str = "openai-gateway/file";
/// Record kind tag designating a gateway batch.
pub const GATEWAY_BATCH_KIND: &str = "openai-gateway/batch";
/// Record kind tag designating a gateway thread.
pub const GATEWAY_THREAD_KIND: &str = "openai-gateway/thread";
/// Record kind tag designating a gateway thread message.
pub const GATEWAY_THREAD_MESSAGE_KIND: &str = "openai-gateway/thread-message";

/// A stored response object linking a response id to its content-addressed
/// [`GatewayResponse`] and the records that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGatewayResponse {
    response_id: String,
    content_id: ContentId,
    response: GatewayResponse,
    pub(crate) request_content_id: Option<ContentId>,
    pub(crate) run_content_id: Option<ContentId>,
    pub(crate) event_content_ids: Vec<ContentId>,
    pub(crate) parent_response_id: Option<String>,
    pub(crate) owner_key_id: Option<String>,
}

impl StoredGatewayResponse {
    /// Creates a stored response with no linked request, run, events, or parent.
    pub fn new(
        response_id: impl Into<String>,
        content_id: ContentId,
        response: GatewayResponse,
    ) -> Self {
        Self {
            response_id: response_id.into(),
            content_id,
            response,
            request_content_id: None,
            run_content_id: None,
            event_content_ids: Vec::new(),
            parent_response_id: None,
            owner_key_id: None,
        }
    }

    /// Returns the public response identifier.
    pub fn response_id(&self) -> &str {
        &self.response_id
    }

    /// Returns the content id of the stored [`GatewayResponse`].
    pub fn content_id(&self) -> &ContentId {
        &self.content_id
    }

    /// Returns the stored response value.
    pub fn response(&self) -> &GatewayResponse {
        &self.response
    }

    /// Returns the gateway key id that created this response, if any.
    pub fn owner_key_id(&self) -> Option<&str> {
        self.owner_key_id.as_deref()
    }
}

/// Store for response objects keyed by their public response id.
pub trait GatewayResponseObjectStore {
    /// Stores a response object, replacing any existing entry with the same id.
    fn put_response_object(&mut self, response: StoredGatewayResponse) -> Result<()>;
    /// Returns the response object with the given id, if present.
    fn response_object(&self, response_id: &str) -> Option<StoredGatewayResponse>;
}

/// Lifecycle status of a [`GatewayBatch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayBatchStatus {
    /// The batch has been accepted but not yet started.
    Queued,
    /// The batch is currently being processed.
    InProgress,
    /// The batch finished processing.
    Completed,
    /// The batch was cancelled before completion.
    Cancelled,
}

impl GatewayBatchStatus {
    /// Returns the OpenAI wire string for this status (e.g. `"in_progress"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Per-request tallies for a [`GatewayBatch`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GatewayBatchCounts {
    total: u64,
    completed: u64,
    failed: u64,
    cancelled: u64,
}

impl GatewayBatchCounts {
    /// Creates a counts record from the total, completed, failed, and cancelled tallies.
    pub fn new(total: u64, completed: u64, failed: u64, cancelled: u64) -> Self {
        Self {
            total,
            completed,
            failed,
            cancelled,
        }
    }

    /// Returns the total number of requests in the batch.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Returns the number of completed requests.
    pub fn completed(&self) -> u64 {
        self.completed
    }

    /// Returns the number of failed requests.
    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// Returns the number of cancelled requests.
    pub fn cancelled(&self) -> u64 {
        self.cancelled
    }

    /// Encodes the counts as a SIM [`Expr`] map record.
    pub fn to_expr(self) -> Expr {
        Expr::Map(vec![
            field("total", Expr::String(self.total.to_string())),
            field("completed", Expr::String(self.completed.to_string())),
            field("failed", Expr::String(self.failed.to_string())),
            field("cancelled", Expr::String(self.cancelled.to_string())),
        ])
    }
}

/// A gateway batch job over an input file, with status and per-request counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayBatch {
    id: String,
    input_file_id: String,
    endpoint: String,
    status: GatewayBatchStatus,
    output_file_id: Option<String>,
    error_file_id: Option<String>,
    created_at_ms: u64,
    completed_at_ms: Option<u64>,
    cancelled_at_ms: Option<u64>,
    request_counts: GatewayBatchCounts,
}

impl GatewayBatch {
    /// Creates a new batch in the [`GatewayBatchStatus::Queued`] state.
    pub fn new(
        id: impl Into<String>,
        input_file_id: impl Into<String>,
        endpoint: impl Into<String>,
        created_at_ms: u64,
        request_counts: GatewayBatchCounts,
    ) -> Self {
        Self {
            id: id.into(),
            input_file_id: input_file_id.into(),
            endpoint: endpoint.into(),
            status: GatewayBatchStatus::Queued,
            output_file_id: None,
            error_file_id: None,
            created_at_ms,
            completed_at_ms: None,
            cancelled_at_ms: None,
            request_counts,
        }
    }

    /// Returns the batch transitioned to [`GatewayBatchStatus::Completed`] with
    /// the given output/error files, completion time, and final counts.
    pub fn complete(
        mut self,
        output_file_id: Option<String>,
        error_file_id: Option<String>,
        completed_at_ms: u64,
        request_counts: GatewayBatchCounts,
    ) -> Self {
        self.status = GatewayBatchStatus::Completed;
        self.output_file_id = output_file_id;
        self.error_file_id = error_file_id;
        self.completed_at_ms = Some(completed_at_ms);
        self.request_counts = request_counts;
        self
    }

    /// Returns the batch transitioned to [`GatewayBatchStatus::Cancelled`],
    /// counting any still-queued requests as cancelled.
    pub fn cancel(mut self, cancelled_at_ms: u64) -> Self {
        let queued = self.request_counts.total.saturating_sub(
            self.request_counts.completed
                + self.request_counts.failed
                + self.request_counts.cancelled,
        );
        self.status = GatewayBatchStatus::Cancelled;
        self.cancelled_at_ms = Some(cancelled_at_ms);
        self.request_counts.cancelled += queued;
        self
    }

    /// Returns the batch identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the id of the input file backing this batch.
    pub fn input_file_id(&self) -> &str {
        &self.input_file_id
    }

    /// Returns the target API endpoint for the batched requests.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Returns the current lifecycle status.
    pub fn status(&self) -> &GatewayBatchStatus {
        &self.status
    }

    /// Returns the output file id, once the batch has produced one.
    pub fn output_file_id(&self) -> Option<&str> {
        self.output_file_id.as_deref()
    }

    /// Returns the error file id, if the batch produced one.
    pub fn error_file_id(&self) -> Option<&str> {
        self.error_file_id.as_deref()
    }

    /// Returns the creation timestamp in milliseconds since the Unix epoch.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Returns the completion timestamp in milliseconds, if completed.
    pub fn completed_at_ms(&self) -> Option<u64> {
        self.completed_at_ms
    }

    /// Returns the cancellation timestamp in milliseconds, if cancelled.
    pub fn cancelled_at_ms(&self) -> Option<u64> {
        self.cancelled_at_ms
    }

    /// Returns the current per-request counts.
    pub fn request_counts(&self) -> GatewayBatchCounts {
        self.request_counts
    }

    /// Encodes the batch as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::String(GATEWAY_BATCH_KIND.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("input-file-id", Expr::String(self.input_file_id.clone())),
            field("endpoint", Expr::String(self.endpoint.clone())),
            field("status", Expr::Symbol(Symbol::new(self.status.as_str()))),
            optional_string_field("output-file-id", self.output_file_id.as_deref()),
            optional_string_field("error-file-id", self.error_file_id.as_deref()),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
            optional_u64_field("completed-at-ms", self.completed_at_ms),
            optional_u64_field("cancelled-at-ms", self.cancelled_at_ms),
            field("request-counts", self.request_counts.to_expr()),
        ])
    }
}

/// Where a [`GatewayFile`]'s bytes live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayFileStorageRef {
    /// Bytes held in an in-memory content-addressed blob.
    Memory {
        /// Content id of the stored bytes.
        content_id: ContentId,
    },
    /// Bytes held on the table-backed filesystem at the given path.
    TableFs {
        /// Filesystem path of the stored bytes.
        path: String,
    },
}

impl GatewayFileStorageRef {
    /// Creates a [`GatewayFileStorageRef::Memory`] reference for `content_id`.
    pub fn memory(content_id: ContentId) -> Self {
        Self::Memory { content_id }
    }

    /// Creates a [`GatewayFileStorageRef::TableFs`] reference for `path`.
    pub fn table_fs(path: impl Into<String>) -> Self {
        Self::TableFs { path: path.into() }
    }

    /// Encodes the storage reference as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        match self {
            Self::Memory { content_id } => Expr::Map(vec![
                field("kind", Expr::Symbol(Symbol::new("memory"))),
                field("content-id", content_id_expr(content_id)),
            ]),
            Self::TableFs { path } => Expr::Map(vec![
                field("kind", Expr::Symbol(Symbol::new("table-fs"))),
                field("path", Expr::String(path.clone())),
            ]),
        }
    }
}

/// A gateway file record: its metadata plus a reference to where its bytes live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayFile {
    id: String,
    filename: String,
    purpose: String,
    bytes: u64,
    created_at_ms: u64,
    storage_ref: GatewayFileStorageRef,
}

impl GatewayFile {
    /// Creates a file record with the given metadata and storage reference.
    pub fn new(
        id: impl Into<String>,
        filename: impl Into<String>,
        purpose: impl Into<String>,
        bytes: u64,
        created_at_ms: u64,
        storage_ref: GatewayFileStorageRef,
    ) -> Self {
        Self {
            id: id.into(),
            filename: filename.into(),
            purpose: purpose.into(),
            bytes,
            created_at_ms,
            storage_ref,
        }
    }

    /// Returns the file identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the original filename.
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Returns the declared purpose of the file (e.g. `"batch"`, `"assistants"`).
    pub fn purpose(&self) -> &str {
        &self.purpose
    }

    /// Returns the file size in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Returns the creation timestamp in milliseconds since the Unix epoch.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Returns a reference to where the file's bytes are stored.
    pub fn storage_ref(&self) -> &GatewayFileStorageRef {
        &self.storage_ref
    }

    /// Encodes the file record as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::String(GATEWAY_FILE_KIND.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("filename", Expr::String(self.filename.clone())),
            field("purpose", Expr::String(self.purpose.clone())),
            field("bytes", Expr::String(self.bytes.to_string())),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
            field("storage-ref", self.storage_ref.to_expr()),
        ])
    }
}

/// A gateway thread record with creation time and key/value metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayThread {
    id: String,
    created_at_ms: u64,
    metadata: Vec<(String, String)>,
}

impl GatewayThread {
    /// Creates a thread record with the given id, creation time, and metadata.
    pub fn new(id: impl Into<String>, created_at_ms: u64, metadata: Vec<(String, String)>) -> Self {
        Self {
            id: id.into(),
            created_at_ms,
            metadata,
        }
    }

    /// Returns the thread identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the creation timestamp in milliseconds since the Unix epoch.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Returns the thread's key/value metadata pairs.
    pub fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }

    /// Encodes the thread record as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::String(GATEWAY_THREAD_KIND.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
            field("metadata", metadata_expr(&self.metadata)),
        ])
    }
}

/// A single message belonging to a [`GatewayThread`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayThreadMessage {
    id: String,
    thread_id: String,
    role: String,
    content: String,
    created_at_ms: u64,
}

impl GatewayThreadMessage {
    /// Creates a thread message with the given id, thread, role, content, and time.
    pub fn new(
        id: impl Into<String>,
        thread_id: impl Into<String>,
        role: impl Into<String>,
        content: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            thread_id: thread_id.into(),
            role: role.into(),
            content: content.into(),
            created_at_ms,
        }
    }

    /// Returns the message identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the id of the thread this message belongs to.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Returns the message role (e.g. `"user"`, `"assistant"`).
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Returns the message text content.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the creation timestamp in milliseconds since the Unix epoch.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Encodes the thread message as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::String(GATEWAY_THREAD_MESSAGE_KIND.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("thread-id", Expr::String(self.thread_id.clone())),
            field("role", Expr::String(self.role.clone())),
            field("content", Expr::String(self.content.clone())),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
        ])
    }
}

/// Store for the gateway's durable account state: files, batches, threads,
/// thread messages, and vector stores, all keyed by their string ids.
pub trait GatewayStateStore {
    /// Stores a file record together with its raw bytes.
    fn put_file(&mut self, file: GatewayFile, bytes: Vec<u8>) -> Result<()>;
    /// Returns the file record with the given id, if present.
    fn file(&self, file_id: &str) -> Option<GatewayFile>;
    /// Returns the raw bytes for the given file id, if present.
    fn file_bytes(&self, file_id: &str) -> Option<Vec<u8>>;

    /// Stores a batch record, replacing any existing entry with the same id.
    fn put_batch(&mut self, batch: GatewayBatch) -> Result<()>;
    /// Returns the batch with the given id, if present.
    fn batch(&self, batch_id: &str) -> Option<GatewayBatch>;

    /// Stores a thread record, replacing any existing entry with the same id.
    fn put_thread(&mut self, thread: GatewayThread) -> Result<()>;
    /// Returns the thread with the given id, if present.
    fn thread(&self, thread_id: &str) -> Option<GatewayThread>;

    /// Appends a message to its thread.
    fn put_thread_message(&mut self, message: GatewayThreadMessage) -> Result<()>;
    /// Returns the messages for the given thread id, in insertion order.
    fn thread_messages(&self, thread_id: &str) -> Vec<GatewayThreadMessage>;

    /// Stores a vector store, replacing any existing entry with the same id.
    fn put_vector_store(&mut self, vector_store: GatewayVectorStore) -> Result<()>;
    /// Returns the vector store with the given id, if present.
    fn vector_store(&self, vector_store_id: &str) -> Option<GatewayVectorStore>;
}

fn metadata_expr(metadata: &[(String, String)]) -> Expr {
    Expr::Map(
        metadata
            .iter()
            .map(|(key, value)| (Expr::String(key.clone()), Expr::String(value.clone())))
            .collect(),
    )
}

use sim_value::build::entry as field;

fn optional_string_field(name: &str, value: Option<&str>) -> (Expr, Expr) {
    field(
        name,
        value
            .map(|value| Expr::String(value.to_owned()))
            .unwrap_or(Expr::Nil),
    )
}

fn optional_u64_field(name: &str, value: Option<u64>) -> (Expr, Expr) {
    field(
        name,
        value
            .map(|value| Expr::String(value.to_string()))
            .unwrap_or(Expr::Nil),
    )
}
