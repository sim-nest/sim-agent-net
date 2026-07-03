//! Storage backends for the OpenAI-compatible gateway.
//!
//! The gateway persists request/run/event/response records plus the durable
//! account state (files, batches, threads, vector stores) behind a set of
//! object-store traits. Concrete backends are provided for in-memory maps
//! ([`MemoryGatewayStore`]) and SIM tables ([`TableGatewayStore`]); both are
//! content-addressed for the eval-pipeline records.

/// In-memory gateway store backed by [`std::collections::BTreeMap`]s.
pub mod memory;
/// Gateway record kinds (files, batches, threads, messages) and their
/// content-addressed object-store traits.
pub mod objects;
/// SIM-table-backed gateway store.
pub mod table;
/// The core [`GatewayStore`] trait over content-addressed eval records.
pub mod r#trait;
/// Vector-store record kinds for the gateway.
pub mod vector;

pub use memory::{GatewayStoreCounts, MemoryGatewayStore};
pub use objects::{
    GATEWAY_BATCH_KIND, GATEWAY_FILE_KIND, GATEWAY_THREAD_KIND, GATEWAY_THREAD_MESSAGE_KIND,
    GatewayBatch, GatewayBatchCounts, GatewayBatchStatus, GatewayFile, GatewayFileStorageRef,
    GatewayResponseObjectStore, GatewayStateStore, GatewayThread, GatewayThreadMessage,
    StoredGatewayResponse,
};
pub use table::TableGatewayStore;
pub use r#trait::GatewayStore;
pub use vector::{
    GATEWAY_VECTOR_STORE_ITEM_KIND, GATEWAY_VECTOR_STORE_KIND, GatewayVectorStore,
    GatewayVectorStoreItem,
};
