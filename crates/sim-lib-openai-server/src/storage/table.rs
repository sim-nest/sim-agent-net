use std::sync::{Arc, Mutex};

use sim_kernel::{
    ContentId, Cx, DefaultFactory, Error, Expr, Factory, NoopEvalPolicy, Object, ObjectCompat,
    Result, Symbol, Table, Value,
};

use crate::{
    objects::{
        GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun, content_id_expr, content_id_hex,
    },
    storage::{
        GatewayBatch, GatewayFile, GatewayResponseObjectStore, GatewayStateStore, GatewayStore,
        GatewayThread, GatewayThreadMessage, GatewayVectorStore, StoredGatewayResponse,
    },
};

/// Gateway store that keeps every record kind in its own SIM [`Table`].
///
/// Each slot is a SIM table value guarded by a shared [`Cx`]; records are
/// wrapped as opaque table values keyed by content id or string id. Implements
/// [`GatewayStore`], [`GatewayResponseObjectStore`], and [`GatewayStateStore`].
pub struct TableGatewayStore {
    cx: Mutex<Cx>,
    requests: Value,
    runs: Value,
    events: Value,
    responses: Value,
    response_objects: Value,
    files: Value,
    file_bytes: Value,
    batches: Value,
    threads: Value,
    thread_messages: Value,
    vector_stores: Value,
}

impl TableGatewayStore {
    /// Creates a store with a fresh table for every record kind.
    ///
    /// Returns an error only if SIM table creation fails, which does not happen
    /// for the in-memory tables used here.
    pub fn new() -> Result<Self> {
        let cx = Mutex::new(Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory)));
        Ok(Self {
            requests: new_table()?,
            runs: new_table()?,
            events: new_table()?,
            responses: new_table()?,
            response_objects: new_table()?,
            files: new_table()?,
            file_bytes: new_table()?,
            batches: new_table()?,
            threads: new_table()?,
            thread_messages: new_table()?,
            vector_stores: new_table()?,
            cx,
        })
    }

    fn set<T>(&self, table: &Value, key: Symbol, value: T) -> Result<()>
    where
        T: TableStored,
    {
        let mut cx = self.cx()?;
        let stored = cx.factory().opaque(Arc::new(StoredTableValue(value)))?;
        table_impl(table)?.set(&mut cx, key, stored)
    }

    fn get<T>(&self, table: &Value, key: Symbol) -> Option<T>
    where
        T: TableStored,
    {
        let mut cx = self.cx().ok()?;
        let value = table_impl(table).ok()?.get(&mut cx, key).ok()?;
        value
            .object()
            .downcast_ref::<StoredTableValue<T>>()
            .map(|stored| stored.0.clone())
    }

    fn cx(&self) -> Result<std::sync::MutexGuard<'_, Cx>> {
        self.cx
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway table store"))
    }
}

impl Default for TableGatewayStore {
    fn default() -> Self {
        Self::new().expect("in-memory SIM table creation is infallible")
    }
}

impl GatewayStore for TableGatewayStore {
    fn put_request(&mut self, id: ContentId, request: GatewayRequest) -> Result<()> {
        self.set(&self.requests, content_key(&id), request)
    }

    fn request(&self, id: &ContentId) -> Option<GatewayRequest> {
        self.get(&self.requests, content_key(id))
    }

    fn put_run(&mut self, id: ContentId, run: GatewayRun) -> Result<()> {
        self.set(&self.runs, content_key(&id), run)
    }

    fn run(&self, id: &ContentId) -> Option<GatewayRun> {
        self.get(&self.runs, content_key(id))
    }

    fn put_event(&mut self, id: ContentId, event: GatewayEvent) -> Result<()> {
        self.set(&self.events, content_key(&id), event)
    }

    fn event(&self, id: &ContentId) -> Option<GatewayEvent> {
        self.get(&self.events, content_key(id))
    }

    fn put_response(&mut self, id: ContentId, response: GatewayResponse) -> Result<()> {
        self.set(&self.responses, content_key(&id), response)
    }

    fn response(&self, id: &ContentId) -> Option<GatewayResponse> {
        self.get(&self.responses, content_key(id))
    }
}

impl GatewayResponseObjectStore for TableGatewayStore {
    fn put_response_object(&mut self, response: StoredGatewayResponse) -> Result<()> {
        self.set(
            &self.responses,
            content_key(response.content_id()),
            response.response().clone(),
        )?;
        self.set(
            &self.response_objects,
            Symbol::new(response.response_id()),
            response,
        )
    }

    fn response_object(&self, response_id: &str) -> Option<StoredGatewayResponse> {
        self.get(&self.response_objects, Symbol::new(response_id))
    }
}

impl GatewayStateStore for TableGatewayStore {
    fn put_file(&mut self, file: GatewayFile, bytes: Vec<u8>) -> Result<()> {
        self.set(&self.file_bytes, Symbol::new(file.id()), bytes)?;
        self.set(&self.files, Symbol::new(file.id()), file)
    }

    fn file(&self, file_id: &str) -> Option<GatewayFile> {
        self.get(&self.files, Symbol::new(file_id))
    }

    fn file_bytes(&self, file_id: &str) -> Option<Vec<u8>> {
        self.get(&self.file_bytes, Symbol::new(file_id))
    }

    fn put_batch(&mut self, batch: GatewayBatch) -> Result<()> {
        self.set(&self.batches, Symbol::new(batch.id()), batch)
    }

    fn batch(&self, batch_id: &str) -> Option<GatewayBatch> {
        self.get(&self.batches, Symbol::new(batch_id))
    }

    fn put_thread(&mut self, thread: GatewayThread) -> Result<()> {
        self.set(&self.threads, Symbol::new(thread.id()), thread)
    }

    fn thread(&self, thread_id: &str) -> Option<GatewayThread> {
        self.get(&self.threads, Symbol::new(thread_id))
    }

    fn put_thread_message(&mut self, message: GatewayThreadMessage) -> Result<()> {
        let key = Symbol::new(message.thread_id());
        let mut messages = self.thread_messages(message.thread_id());
        messages.push(message);
        self.set(&self.thread_messages, key, ThreadMessages(messages))
    }

    fn thread_messages(&self, thread_id: &str) -> Vec<GatewayThreadMessage> {
        self.get::<ThreadMessages>(&self.thread_messages, Symbol::new(thread_id))
            .map(|messages| messages.0)
            .unwrap_or_default()
    }

    fn put_vector_store(&mut self, vector_store: GatewayVectorStore) -> Result<()> {
        self.set(
            &self.vector_stores,
            Symbol::new(vector_store.id()),
            vector_store,
        )
    }

    fn vector_store(&self, vector_store_id: &str) -> Option<GatewayVectorStore> {
        self.get(&self.vector_stores, Symbol::new(vector_store_id))
    }
}

trait TableStored: Clone + Send + Sync + 'static {
    fn label() -> &'static str;
    fn to_expr(&self) -> Expr;
}

#[sim_citizen_derive::non_citizen(
    reason = "OpenAI table storage wrapper; class-backed descriptor is the wrapped openai/* value",
    kind = "marker"
)]
#[derive(Clone)]
struct StoredTableValue<T: TableStored>(T);

impl<T: TableStored> Object for StoredTableValue<T> {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<{}>", T::label()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<T: TableStored> ObjectCompat for StoredTableValue<T> {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.0.to_expr())
    }
}

#[derive(Clone)]
struct ThreadMessages(Vec<GatewayThreadMessage>);

impl TableStored for GatewayRequest {
    fn label() -> &'static str {
        "openai-gateway-table-request"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayRun {
    fn label() -> &'static str {
        "openai-gateway-table-run"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayEvent {
    fn label() -> &'static str {
        "openai-gateway-table-event"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayResponse {
    fn label() -> &'static str {
        "openai-gateway-table-response"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for StoredGatewayResponse {
    fn label() -> &'static str {
        "openai-gateway-table-stored-response"
    }

    fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("response-id", Expr::String(self.response_id().to_owned())),
            field("content-id", content_id_expr(self.content_id())),
            field("response", self.response().to_expr()),
            optional_content_id_field("request-content-id", self.request_content_id.as_ref()),
            optional_content_id_field("run-content-id", self.run_content_id.as_ref()),
            field(
                "event-content-ids",
                Expr::Vector(
                    self.event_content_ids
                        .iter()
                        .map(content_id_expr)
                        .collect::<Vec<_>>(),
                ),
            ),
            field(
                "parent-response-id",
                self.parent_response_id
                    .as_ref()
                    .map(|id| Expr::String(id.clone()))
                    .unwrap_or(Expr::Nil),
            ),
        ])
    }
}

impl TableStored for GatewayFile {
    fn label() -> &'static str {
        "openai-gateway-table-file"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for Vec<u8> {
    fn label() -> &'static str {
        "openai-gateway-table-bytes"
    }

    fn to_expr(&self) -> Expr {
        Expr::Bytes(self.clone())
    }
}

impl TableStored for GatewayBatch {
    fn label() -> &'static str {
        "openai-gateway-table-batch"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayThread {
    fn label() -> &'static str {
        "openai-gateway-table-thread"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayThreadMessage {
    fn label() -> &'static str {
        "openai-gateway-table-thread-message"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for GatewayVectorStore {
    fn label() -> &'static str {
        "openai-gateway-table-vector-store"
    }

    fn to_expr(&self) -> Expr {
        self.to_expr()
    }
}

impl TableStored for ThreadMessages {
    fn label() -> &'static str {
        "openai-gateway-table-thread-messages"
    }

    fn to_expr(&self) -> Expr {
        Expr::Vector(
            self.0
                .iter()
                .map(GatewayThreadMessage::to_expr)
                .collect::<Vec<_>>(),
        )
    }
}

fn new_table() -> Result<Value> {
    DefaultFactory.table(Vec::new())
}

fn table_impl(value: &Value) -> Result<&dyn Table> {
    value
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval("gateway table store slot is not a table".to_owned()))
}

fn content_key(id: &ContentId) -> Symbol {
    Symbol::new(content_id_hex(id))
}

use sim_value::build::entry as field;

fn optional_content_id_field(name: &str, value: Option<&ContentId>) -> (Expr, Expr) {
    field(name, value.map(content_id_expr).unwrap_or(Expr::Nil))
}
