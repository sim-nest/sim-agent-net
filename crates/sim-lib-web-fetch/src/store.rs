use crate::{CaptureDir, FetchError, StoredCapture, StoredRobots};
use sim_kernel::ContentId;
use std::{collections::BTreeMap, sync::Mutex};

/// Deterministic caller-owned fixture store.
#[derive(Default)]
pub struct MemoryCaptureDir {
    captures: Mutex<BTreeMap<ContentId, StoredCapture>>,
    urls: Mutex<BTreeMap<String, ContentId>>,
    robots: Mutex<BTreeMap<String, StoredRobots>>,
}
impl CaptureDir for MemoryCaptureDir {
    fn capture(&self, id: &ContentId) -> Result<Option<StoredCapture>, FetchError> {
        Ok(self
            .captures
            .lock()
            .map_err(|_| FetchError::Storage("capture lock poisoned".into()))?
            .get(id)
            .cloned())
    }
    fn put_capture(&self, v: StoredCapture) -> Result<(), FetchError> {
        let id = v.receipt.capture.content_id.clone();
        let mut values = self
            .captures
            .lock()
            .map_err(|_| FetchError::Storage("capture lock poisoned".into()))?;
        if let Some(old) = values.get(&id) {
            if old.receipt.capture.body != v.receipt.capture.body {
                return Err(FetchError::Storage("immutable capture collision".into()));
            }
            return Ok(());
        }
        values.insert(id, v);
        Ok(())
    }
    fn url_capture(&self, url: &str) -> Result<Option<ContentId>, FetchError> {
        Ok(self
            .urls
            .lock()
            .map_err(|_| FetchError::Storage("url lock poisoned".into()))?
            .get(url)
            .cloned())
    }
    fn point_url(&self, url: &str, id: &ContentId) -> Result<(), FetchError> {
        self.urls
            .lock()
            .map_err(|_| FetchError::Storage("url lock poisoned".into()))?
            .insert(url.into(), id.clone());
        Ok(())
    }
    fn robots(&self, origin: &str) -> Result<Option<StoredRobots>, FetchError> {
        Ok(self
            .robots
            .lock()
            .map_err(|_| FetchError::Storage("robots lock poisoned".into()))?
            .get(origin)
            .cloned())
    }
    fn put_robots(&self, origin: &str, value: StoredRobots) -> Result<(), FetchError> {
        self.robots
            .lock()
            .map_err(|_| FetchError::Storage("robots lock poisoned".into()))?
            .insert(origin.into(), value);
        Ok(())
    }
}
