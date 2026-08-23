use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

use serde_json::Value;

use crate::{ClientError, EndpointIdentity, Era, PersistentCache};

/// Complete canonical cache identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CacheKey {
    /// Endpoint without credentials.
    pub endpoint: EndpointIdentity,
    /// Negotiated era.
    pub era: Era,
    /// Negotiated protocol version.
    pub version: String,
    /// Authenticated principal/cache scope.
    pub principal_scope: String,
    /// Negotiated extensions.
    pub extensions: Vec<String>,
    /// MCP method.
    pub method: String,
    /// Canonical JSON parameters.
    pub canonical_parameters: String,
    /// Canonical pagination cursor.
    pub pagination_cursor: Option<String>,
}

impl CacheKey {
    /// Redaction-safe stable serialization for injected persistent stores.
    pub fn stable(&self) -> String {
        format!(
            "{}|{:?}|{}|{}|{}|{}|{}|{}",
            self.endpoint,
            self.era,
            self.version,
            self.principal_scope,
            self.extensions.join(","),
            self.method,
            self.canonical_parameters,
            self.pagination_cursor.as_deref().unwrap_or("")
        )
    }
}

/// Cache lookup disposition.
#[derive(Clone, Debug, PartialEq)]
pub enum CacheDisposition {
    /// Complete unexpired hit.
    Hit(Value),
    /// No complete unexpired entry.
    Miss,
}

/// Cache interface used by the client.
pub trait ClientCache: Send + Sync {
    /// Reads an unexpired complete value.
    fn get(&self, key: &CacheKey, now_ms: u64) -> Result<CacheDisposition, ClientError>;
    /// Stores one complete eligible value.
    fn put(&self, key: CacheKey, value: Value, expires_at_ms: u64) -> Result<(), ClientError>;
    /// Clears entries for an endpoint after identity/token-scope change.
    fn clear_endpoint(&self, endpoint: &EndpointIdentity) -> Result<(), ClientError>;
}

struct Entry {
    value: Value,
    expires_at_ms: u64,
}
struct MemoryState {
    rows: BTreeMap<CacheKey, Entry>,
    order: VecDeque<CacheKey>,
}

/// Bounded in-memory LRU, optionally mirrored through a host-owned persistent store.
pub struct MemoryLruCache {
    capacity: usize,
    state: Mutex<MemoryState>,
    persistent: Option<Box<dyn PersistentCache>>,
}

impl MemoryLruCache {
    /// Constructs a non-zero bounded cache.
    pub fn new(capacity: usize) -> Result<Self, ClientError> {
        if capacity == 0 {
            return Err(ClientError::Cache("LRU capacity must be non-zero".into()));
        }
        Ok(Self {
            capacity,
            state: Mutex::new(MemoryState {
                rows: BTreeMap::new(),
                order: VecDeque::new(),
            }),
            persistent: None,
        })
    }
    /// Attaches a persistent store whose encryption/privacy policy remains host owned.
    pub fn with_persistent(mut self, persistent: Box<dyn PersistentCache>) -> Self {
        self.persistent = Some(persistent);
        self
    }
}

impl ClientCache for MemoryLruCache {
    fn get(&self, key: &CacheKey, now_ms: u64) -> Result<CacheDisposition, ClientError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ClientError::Cache("poisoned LRU".into()))?;
        if state
            .rows
            .get(key)
            .is_some_and(|entry| entry.expires_at_ms <= now_ms)
        {
            state.rows.remove(key);
        }
        if let Some(value) = state.rows.get(key).map(|entry| entry.value.clone()) {
            state.order.retain(|candidate| candidate != key);
            state.order.push_back(key.clone());
            return Ok(CacheDisposition::Hit(value));
        }
        drop(state);
        if let Some(store) = &self.persistent {
            if let Some(bytes) = store.get(&key.stable(), now_ms)? {
                let value = serde_json::from_slice(&bytes)
                    .map_err(|error| ClientError::Cache(error.to_string()))?;
                return Ok(CacheDisposition::Hit(value));
            }
        }
        Ok(CacheDisposition::Miss)
    }
    fn put(&self, key: CacheKey, value: Value, expires_at_ms: u64) -> Result<(), ClientError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ClientError::Cache("poisoned LRU".into()))?;
        state.order.retain(|candidate| candidate != &key);
        state.order.push_back(key.clone());
        state.rows.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                expires_at_ms,
            },
        );
        while state.rows.len() > self.capacity {
            if let Some(oldest) = state.order.pop_front() {
                state.rows.remove(&oldest);
            }
        }
        drop(state);
        if let Some(store) = &self.persistent {
            let bytes = serde_json::to_vec(&value)
                .map_err(|error| ClientError::Cache(error.to_string()))?;
            store.put(&key.stable(), &bytes, expires_at_ms)?;
        }
        Ok(())
    }
    fn clear_endpoint(&self, endpoint: &EndpointIdentity) -> Result<(), ClientError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ClientError::Cache("poisoned LRU".into()))?;
        state.rows.retain(|key, _| &key.endpoint != endpoint);
        state.order.retain(|key| &key.endpoint != endpoint);
        drop(state);
        if let Some(store) = &self.persistent {
            store.clear_private(endpoint)?;
        }
        Ok(())
    }
}
