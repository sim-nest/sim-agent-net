use crate::{Failure, Limits, PhaseId};

/// Root-to-subject path attached to every structural policy failure.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CausalPath(Vec<PhaseId>);

impl CausalPath {
    pub(crate) fn new(parts: Vec<PhaseId>, limits: Limits) -> Result<Self, Failure> {
        if parts.len() > limits.causal_path {
            return Err(Failure::OverLimit {
                limit: "causal_path",
                actual: parts.len(),
                maximum: limits.causal_path,
            });
        }
        Ok(Self(parts))
    }

    pub fn phases(&self) -> &[PhaseId] {
        &self.0
    }
}
