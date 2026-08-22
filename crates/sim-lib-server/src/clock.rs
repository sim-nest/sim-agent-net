use std::sync::atomic::{AtomicU64, Ordering};

use sim_kernel::{Error, Result};

/// One observed host wall-clock instant, represented as milliseconds since the Unix epoch.
///
/// A wall timestamp is human-facing evidence and a scheduling input. It is not
/// monotonic: callers must use logical ticks, revisions, or [`std::time::Instant`]
/// when correctness depends on ordering or elapsed time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallTimestamp(u64);

impl WallTimestamp {
    /// Creates an explicit Unix-millisecond wall-clock observation.
    pub const fn from_unix_millis(unix_millis: u64) -> Self {
        Self(unix_millis)
    }

    /// Returns the observed Unix milliseconds.
    pub const fn unix_millis(self) -> u64 {
        self.0
    }
}

/// Object-safe source of host wall-clock observations.
///
/// Implementations may move backward between observations. Code that needs a
/// correctness clock must instead use a logical tick, revision, or monotonic
/// deadline.
pub trait WallClock: Send + Sync {
    /// Observes the current host wall-clock timestamp.
    fn now(&self) -> Result<WallTimestamp>;

    /// Observes the current host wall clock as Unix milliseconds.
    fn now_ms(&self) -> Result<u64> {
        self.now().map(WallTimestamp::unix_millis)
    }
}

/// Thread-safe [`WallClock`] that advances by a fixed step on each observation.
///
/// The clock starts at `start_ms` and advances by `step_ms` after every read,
/// producing reproducible observations without consulting ambient host time.
#[derive(Debug)]
pub struct DeterministicWallClock {
    next_ms: AtomicU64,
    step_ms: u64,
}

impl DeterministicWallClock {
    /// Builds a deterministic clock with an initial Unix-millisecond value and fixed step.
    pub const fn new(start_ms: u64, step_ms: u64) -> Self {
        Self {
            next_ms: AtomicU64::new(start_ms),
            step_ms,
        }
    }
}

impl Clone for DeterministicWallClock {
    fn clone(&self) -> Self {
        Self::new(self.next_ms.load(Ordering::Acquire), self.step_ms)
    }
}

impl WallClock for DeterministicWallClock {
    fn now(&self) -> Result<WallTimestamp> {
        self.next_ms
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(self.step_ms)
            })
            .map(WallTimestamp::from_unix_millis)
            .map_err(|_| Error::Eval("deterministic wall clock overflow".to_owned()))
    }
}

#[cfg(test)]
mod tests;
