use sha2::{Digest, Sha256};
use sim_lib_journal::StoreError;

use crate::{ExecutionJournalError, ExecutionPins};

pub(crate) fn store_error(error: StoreError) -> ExecutionJournalError {
    match error {
        StoreError::Journal(error) => error.into(),
    }
}
pub(crate) fn contains_secret(bytes: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    [
        "api_key=",
        "api-key:",
        "authorization: bearer ",
        "password=",
        "private key-----",
        "secret=",
    ]
    .iter()
    .any(|p| lower.contains(p))
}
pub(crate) fn bounded_summary(value: &str) -> String {
    value.chars().take(240).collect()
}
pub(crate) fn child_id(parent: &str, pins: &ExecutionPins) -> String {
    let mut h = Sha256::new();
    h.update(b"sim-roadmap-child-v1\0");
    for s in [
        parent,
        &pins.conduct,
        &pins.policy,
        &pins.model_pick,
        &pins.runner_generation,
        &pins.source_deck.algorithm.as_qualified_str(),
    ] {
        h.update((s.len() as u64).to_be_bytes());
        h.update(s.as_bytes());
    }
    h.update(pins.source_deck.bytes);
    format!("{parent}-child-{:x}", h.finalize())
}
