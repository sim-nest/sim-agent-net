use std::collections::BTreeMap;

use sim_kernel::{ContentId, Result};

use crate::{
    content_id::{content_id_for_expr, request_content_id},
    objects::{GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun},
    storage::{
        GatewayBatch, GatewayFile, GatewayResponseObjectStore, GatewayStateStore, GatewayStore,
        GatewayThread, GatewayThreadMessage, GatewayVectorStore, StoredGatewayResponse,
    },
};

/// Snapshot of how many records of each kind a gateway store currently holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GatewayStoreCounts {
    /// Number of stored requests.
    pub request_count: usize,
    /// Number of stored runs.
    pub run_count: usize,
    /// Number of stored events.
    pub event_count: usize,
    /// Number of stored responses.
    pub response_count: usize,
    /// Number of stored response objects.
    pub response_object_count: usize,
    /// Number of stored file records.
    pub file_count: usize,
    /// Number of stored file byte blobs.
    pub file_bytes_count: usize,
    /// Number of stored batches.
    pub batch_count: usize,
    /// Number of stored threads.
    pub thread_count: usize,
    /// Total number of stored thread messages across all threads.
    pub thread_message_count: usize,
    /// Number of stored vector stores.
    pub vector_store_count: usize,
}

/// In-memory gateway store backed by [`BTreeMap`]s.
///
/// Implements [`GatewayStore`], [`GatewayResponseObjectStore`], and
/// [`GatewayStateStore`]. Intended for tests and single-process use; nothing is
/// persisted beyond the process.
#[derive(Clone, Debug, Default)]
pub struct MemoryGatewayStore {
    requests: BTreeMap<ContentId, GatewayRequest>,
    runs: BTreeMap<ContentId, GatewayRun>,
    events: BTreeMap<ContentId, GatewayEvent>,
    responses: BTreeMap<ContentId, GatewayResponse>,
    response_objects: BTreeMap<String, StoredGatewayResponse>,
    files: BTreeMap<String, GatewayFile>,
    file_bytes: BTreeMap<String, Vec<u8>>,
    batches: BTreeMap<String, GatewayBatch>,
    threads: BTreeMap<String, GatewayThread>,
    thread_messages: BTreeMap<String, Vec<GatewayThreadMessage>>,
    vector_stores: BTreeMap<String, GatewayVectorStore>,
}

impl MemoryGatewayStore {
    /// Creates an empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a request under its derived content id and returns that id.
    pub fn put_request_content(&mut self, request: GatewayRequest) -> Result<ContentId> {
        let id = request_content_id(&request)?;
        self.put_request(id.clone(), request)?;
        Ok(id)
    }

    /// Stores a run under its derived content id and returns that id.
    pub fn put_run_content(&mut self, run: GatewayRun) -> Result<ContentId> {
        let id = content_id_for_expr(&run.to_expr())?;
        self.put_run(id.clone(), run)?;
        Ok(id)
    }

    /// Stores an event under its derived content id and returns that id.
    pub fn put_event_content(&mut self, event: GatewayEvent) -> Result<ContentId> {
        let id = content_id_for_expr(&event.to_expr())?;
        self.put_event(id.clone(), event)?;
        Ok(id)
    }

    /// Stores a response under its derived content id and returns that id.
    pub fn put_response_content(&mut self, response: GatewayResponse) -> Result<ContentId> {
        let id = content_id_for_expr(&response.to_expr())?;
        self.put_response(id.clone(), response)?;
        Ok(id)
    }

    /// Returns a snapshot of the current record counts.
    pub fn counts(&self) -> GatewayStoreCounts {
        GatewayStoreCounts {
            request_count: self.requests.len(),
            run_count: self.runs.len(),
            event_count: self.events.len(),
            response_count: self.responses.len(),
            response_object_count: self.response_objects.len(),
            file_count: self.files.len(),
            file_bytes_count: self.file_bytes.len(),
            batch_count: self.batches.len(),
            thread_count: self.threads.len(),
            thread_message_count: self.thread_messages.values().map(Vec::len).sum(),
            vector_store_count: self.vector_stores.len(),
        }
    }

    /// Returns all stored requests paired with their content ids.
    pub fn requests(&self) -> Vec<(ContentId, GatewayRequest)> {
        self.requests
            .iter()
            .map(|(id, request)| (id.clone(), request.clone()))
            .collect()
    }

    /// Returns all stored runs paired with their content ids.
    pub fn runs(&self) -> Vec<(ContentId, GatewayRun)> {
        self.runs
            .iter()
            .map(|(id, run)| (id.clone(), run.clone()))
            .collect()
    }

    /// Returns the run (with its content id) whose run id matches `run_id`.
    pub fn run_by_id(&self, run_id: &str) -> Option<(ContentId, GatewayRun)> {
        self.runs
            .iter()
            .find_map(|(id, run)| (run.id() == run_id).then(|| (id.clone(), run.clone())))
    }

    /// Returns all stored events paired with their content ids.
    pub fn events(&self) -> Vec<(ContentId, GatewayEvent)> {
        self.events
            .iter()
            .map(|(id, event)| (id.clone(), event.clone()))
            .collect()
    }
}

impl GatewayStore for MemoryGatewayStore {
    fn put_request(&mut self, id: ContentId, request: GatewayRequest) -> Result<()> {
        self.requests.insert(id, request);
        Ok(())
    }

    fn request(&self, id: &ContentId) -> Option<GatewayRequest> {
        self.requests.get(id).cloned()
    }

    fn put_run(&mut self, id: ContentId, run: GatewayRun) -> Result<()> {
        self.runs.insert(id, run);
        Ok(())
    }

    fn run(&self, id: &ContentId) -> Option<GatewayRun> {
        self.runs.get(id).cloned()
    }

    fn put_event(&mut self, id: ContentId, event: GatewayEvent) -> Result<()> {
        self.events.insert(id, event);
        Ok(())
    }

    fn event(&self, id: &ContentId) -> Option<GatewayEvent> {
        self.events.get(id).cloned()
    }

    fn put_response(&mut self, id: ContentId, response: GatewayResponse) -> Result<()> {
        self.responses.insert(id, response);
        Ok(())
    }

    fn response(&self, id: &ContentId) -> Option<GatewayResponse> {
        self.responses.get(id).cloned()
    }
}

impl GatewayResponseObjectStore for MemoryGatewayStore {
    fn put_response_object(&mut self, response: StoredGatewayResponse) -> Result<()> {
        self.responses
            .insert(response.content_id().clone(), response.response().clone());
        self.response_objects
            .insert(response.response_id().to_owned(), response);
        Ok(())
    }

    fn response_object(&self, response_id: &str) -> Option<StoredGatewayResponse> {
        self.response_objects.get(response_id).cloned()
    }
}

impl GatewayStateStore for MemoryGatewayStore {
    fn put_file(&mut self, file: GatewayFile, bytes: Vec<u8>) -> Result<()> {
        self.file_bytes.insert(file.id().to_owned(), bytes);
        self.files.insert(file.id().to_owned(), file);
        Ok(())
    }

    fn file(&self, file_id: &str) -> Option<GatewayFile> {
        self.files.get(file_id).cloned()
    }

    fn file_bytes(&self, file_id: &str) -> Option<Vec<u8>> {
        self.file_bytes.get(file_id).cloned()
    }

    fn put_batch(&mut self, batch: GatewayBatch) -> Result<()> {
        self.batches.insert(batch.id().to_owned(), batch);
        Ok(())
    }

    fn batch(&self, batch_id: &str) -> Option<GatewayBatch> {
        self.batches.get(batch_id).cloned()
    }

    fn put_thread(&mut self, thread: GatewayThread) -> Result<()> {
        self.threads.insert(thread.id().to_owned(), thread);
        Ok(())
    }

    fn thread(&self, thread_id: &str) -> Option<GatewayThread> {
        self.threads.get(thread_id).cloned()
    }

    fn put_thread_message(&mut self, message: GatewayThreadMessage) -> Result<()> {
        self.thread_messages
            .entry(message.thread_id().to_owned())
            .or_default()
            .push(message);
        Ok(())
    }

    fn thread_messages(&self, thread_id: &str) -> Vec<GatewayThreadMessage> {
        self.thread_messages
            .get(thread_id)
            .cloned()
            .unwrap_or_default()
    }

    fn put_vector_store(&mut self, vector_store: GatewayVectorStore) -> Result<()> {
        self.vector_stores
            .insert(vector_store.id().to_owned(), vector_store);
        Ok(())
    }

    fn vector_store(&self, vector_store_id: &str) -> Option<GatewayVectorStore> {
        self.vector_stores.get(vector_store_id).cloned()
    }
}
