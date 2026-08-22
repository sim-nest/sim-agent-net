use crate::{ProviderFamilyCard, ProviderSeatCard};
use sim_kernel::{Cx, Expr, Result};
use sim_lib_agent_runner_core::ModelRunner;
use std::sync::Arc;

/// Setup-only contract for discovering and opening provider seats.
///
/// Inference is intentionally absent: an opened seat is an ordinary
/// [`ModelRunner`].
pub trait ProviderAdapter: Send + Sync {
    /// Describes the provider family implemented by this adapter.
    fn family(&self) -> ProviderFamilyCard;

    /// Enumerates every seat this adapter can see. Returning a merged or
    /// preferred subset is a defect.
    fn discover(&self, cx: &mut Cx, hint: Expr) -> Result<Vec<ProviderSeatCard>>;

    /// Opens exactly one seat as an ordinary [`ModelRunner`].
    fn open(
        &self,
        cx: &mut Cx,
        seat: &ProviderSeatCard,
        options: Expr,
    ) -> Result<Arc<dyn ModelRunner>>;
}
