use crate::{ProviderFamilyCard, ProviderRegistry, ProviderSeatCard, ProviderSeatId};
use sim_kernel::{Cx, Expr, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use std::sync::Arc;

/// Implements the pure `provider/families` control operation.
pub fn families(registry: &ProviderRegistry) -> Vec<ProviderFamilyCard> {
    registry.families()
}

/// Implements the pure `provider/seats` control operation.
pub fn seats(registry: &ProviderRegistry) -> Vec<ProviderSeatCard> {
    registry.seats()
}

/// Implements the pure `provider/show-family` control operation.
pub fn show_family(registry: &ProviderRegistry, family: &Symbol) -> Option<ProviderFamilyCard> {
    registry.show_family(family)
}

/// Implements the pure `provider/show-seat` control operation.
pub fn show_seat(registry: &ProviderRegistry, seat: &ProviderSeatId) -> Option<ProviderSeatCard> {
    registry.show_seat(seat)
}

/// Implements adapter-driven `provider/discover` without vendor knowledge.
pub fn discover(
    registry: &mut ProviderRegistry,
    cx: &mut Cx,
    hint: Expr,
) -> Result<Vec<ProviderSeatCard>> {
    registry.discover(cx, hint)
}

/// Implements adapter-driven `provider/open` for one explicit seat.
pub fn open(
    registry: &ProviderRegistry,
    cx: &mut Cx,
    seat: &ProviderSeatId,
    options: Expr,
) -> Result<Arc<dyn ModelRunner>> {
    registry.open(cx, seat, options)
}
