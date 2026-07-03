use std::sync::Arc;

use sim_kernel::{Cx, Result};

use crate::{FixtureTransport, SkillCard, SkillLib};

/// Installs the skill library into `cx` once and publishes its browse metadata.
pub fn install_skill_lib(cx: &mut Cx) -> Result<()> {
    sim_lib_core::install_once(cx, &SkillLib)?;
    crate::browse::publish_skill_browse_metadata(cx)
}

/// Installs the skill library, registers `transport`, and binds `card`.
///
/// A convenience for tests and examples that wires a [`FixtureTransport`] and
/// a single [`SkillCard`] into a freshly prepared registry.
pub fn install_fixture_skill(
    cx: &mut Cx,
    transport: Arc<FixtureTransport>,
    card: SkillCard,
) -> Result<()> {
    install_skill_lib(cx)?;
    let registry = crate::skill_registry(cx)?;
    registry.install_transport(transport)?;
    registry.bind_card(cx, card)?;
    Ok(())
}
