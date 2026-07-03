use std::time::{SystemTime, UNIX_EPOCH};

use sim_kernel::{Error, Result};

/// Source of millisecond wall-clock timestamps for the gateway.
///
/// Abstracts time so production code can use the real system clock while tests
/// use a reproducible deterministic clock.
pub trait GatewayClock {
    /// Returns the current time in milliseconds since the Unix epoch.
    fn now_ms(&mut self) -> Result<u64>;
}

/// [`GatewayClock`] backed by the real system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemGatewayClock;

impl GatewayClock for SystemGatewayClock {
    fn now_ms(&mut self) -> Result<u64> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| Error::Eval(format!("system clock is before UNIX_EPOCH: {err}")))?;
        u64::try_from(duration.as_millis())
            .map_err(|_| Error::Eval("system timestamp exceeds u64 milliseconds".to_owned()))
    }
}

/// [`GatewayClock`] that advances by a fixed step on each read.
///
/// Starts at `start_ms` and adds `step_ms` after every [`GatewayClock::now_ms`]
/// call, producing reproducible timestamps for tests.
#[derive(Clone, Debug)]
pub struct DeterministicGatewayClock {
    next_ms: u64,
    step_ms: u64,
}

impl DeterministicGatewayClock {
    /// Builds a clock that starts at `start_ms` and advances by `step_ms` per
    /// read.
    pub fn new(start_ms: u64, step_ms: u64) -> Self {
        Self {
            next_ms: start_ms,
            step_ms,
        }
    }
}

impl GatewayClock for DeterministicGatewayClock {
    fn now_ms(&mut self) -> Result<u64> {
        let now = self.next_ms;
        self.next_ms = self
            .next_ms
            .checked_add(self.step_ms)
            .ok_or_else(|| Error::Eval("deterministic gateway clock overflow".to_owned()))?;
        Ok(now)
    }
}
