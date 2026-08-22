use crate::{
    AuthMethod, ProviderControlResult, ProviderFamilyCard, ProviderSeatCard, SessionStatus,
};
use sim_kernel::{Cx, Error, Expr, Result};
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

    /// Lists supported authentication methods without performing inference.
    fn auth_methods(&self, _cx: &mut Cx) -> Result<Vec<AuthMethod>> {
        Ok(Vec::new())
    }

    /// Starts or resumes a typed login flow.
    fn login(
        &self,
        _cx: &mut Cx,
        _seat: &ProviderSeatCard,
        _method: AuthMethod,
    ) -> Result<SessionStatus> {
        Err(Error::Eval("provider login is not supported".into()))
    }

    /// Queries current session state without performing inference.
    fn status(&self, _cx: &mut Cx, _seat: &ProviderSeatCard) -> Result<SessionStatus> {
        Ok(SessionStatus::LoginRequired)
    }

    /// Ends the current provider session.
    fn logout(&self, _cx: &mut Cx, _seat: &ProviderSeatCard) -> Result<ProviderControlResult> {
        Err(Error::Eval("provider logout is not supported".into()))
    }
}
