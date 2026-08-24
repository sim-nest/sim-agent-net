use crate::PlanKey;
use sim_incremental_core::{IncrementalEngine, ObservationKind};

/// Thin domain adapter; generic memo and reverse-edge ownership stays in sim-incremental-core.
pub struct DependencyIndex {
    engine: IncrementalEngine<PlanKey, String>,
}

impl DependencyIndex {
    pub fn new() -> Self {
        Self {
            engine: IncrementalEngine::new(),
        }
    }
    pub fn register_observed(&mut self, key: PlanKey, facts: Vec<PlanKey>, value: String) {
        self.engine.register_fn(key, move |_, frame| {
            for fact in &facts {
                frame.observe(ObservationKind::Custom("roadmap-fact"), fact.clone())?;
            }
            Ok(value.clone())
        });
    }
    pub fn verify(&mut self, key: PlanKey) -> Result<String, String> {
        self.engine.verify(key).map_err(|e| format!("{e:?}"))
    }
    pub fn invalidate(&mut self, fact: &PlanKey) -> Vec<PlanKey> {
        self.engine.invalidate(fact);
        self.engine.dirty_keys()
    }
}
impl Default for DependencyIndex {
    fn default() -> Self {
        Self::new()
    }
}
