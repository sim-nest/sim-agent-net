use sim_citizen_derive::Citizen;
use sim_kernel::{CapabilityName, Symbol};

/// Serializable `skill/Card` projection of a [`SkillCard`].
///
/// Where [`SkillCard`] is a shape-bearing runtime handle, this descriptor is a
/// plain citizen value carrying only the serializable metadata, suitable for
/// transport across the runtime boundary.
///
/// [`SkillCard`]: crate::SkillCard
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "skill/Card", version = 1)]
pub struct SkillCardDescriptor {
    /// Stable string identifier for the skill.
    pub id: String,
    /// Symbol the skill is registered and called under.
    pub symbol: Symbol,
    /// Human-readable title.
    pub title: String,
    /// Human-readable description.
    pub description: String,
    /// Symbols naming the roles the skill plays.
    pub roles: Vec<Symbol>,
    /// Capabilities a caller must hold to invoke the skill.
    pub capabilities: Vec<CapabilityName>,
    /// Identifier of the transport that runs the skill.
    pub transport_id: String,
    /// Kind of the transport.
    pub transport_kind: String,
    /// Operation name the transport dispatches.
    pub operation: String,
}

impl Default for SkillCardDescriptor {
    fn default() -> Self {
        Self {
            id: "citizen-skill".to_owned(),
            symbol: Symbol::qualified("skill", "citizen-skill"),
            title: "Citizen Skill".to_owned(),
            description: "Citizen fixture skill descriptor".to_owned(),
            roles: vec![Symbol::new("tool")],
            capabilities: vec![CapabilityName::new("skill.call.citizen-skill")],
            transport_id: "fixture".to_owned(),
            transport_kind: "fixture".to_owned(),
            operation: "citizen-skill".to_owned(),
        }
    }
}

/// Returns the class symbol (`skill/Card`) for [`SkillCardDescriptor`].
pub fn skill_card_descriptor_class_symbol() -> Symbol {
    Symbol::qualified("skill", "Card")
}
