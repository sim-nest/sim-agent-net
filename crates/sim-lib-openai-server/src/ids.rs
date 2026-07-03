use sim_kernel::{Error, Result};

/// Generates sequential, prefixed gateway ids of the form `prefix_NNNNNN`.
///
/// Counting is deterministic: ids advance monotonically from a fixed start,
/// which makes generated ids reproducible across runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayIdGenerator {
    prefix: String,
    next: u64,
}

impl GatewayIdGenerator {
    /// Builds a generator that emits ids with the given `prefix`, starting the
    /// counter at `start`.
    pub fn deterministic(prefix: impl Into<String>, start: u64) -> Self {
        Self {
            prefix: prefix.into(),
            next: start,
        }
    }

    /// Returns the next id and advances the counter, erroring if the counter
    /// would overflow `u64`.
    pub fn next_id(&mut self) -> Result<String> {
        let value = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::Eval("gateway id counter overflow".to_owned()))?;
        Ok(format!("{}_{value:06}", self.prefix))
    }
}
