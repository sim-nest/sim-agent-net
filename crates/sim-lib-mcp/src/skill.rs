use sim_kernel::{Cx, Result};
use sim_lib_skill::{SkillCard, SkillRole, skill_registry};

use crate::surface::stable_mcp_name_text;
use crate::{
    McpProfile, McpStreamPolicy, McpSurfaceCard, McpSurfaceRole, McpSurfaceSource,
    project_surface_rows,
};

/// Projects the registered skill cards into surface rows filtered by `profile`.
pub fn project_skill_surface(cx: &mut Cx, profile: &McpProfile) -> Result<Vec<McpSurfaceCard>> {
    project_surface_rows(skill_surface_rows(cx)?, profile)
}

/// Builds unfiltered surface rows from the skill registry in `cx`.
///
/// Returns an empty list when no skill registry is installed.
pub fn skill_surface_rows(cx: &mut Cx) -> Result<Vec<McpSurfaceCard>> {
    let Ok(registry) = skill_registry(cx) else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for card in registry.cards()? {
        for role in card.roles.iter().filter_map(skill_role_to_mcp) {
            rows.push(row_from_skill(&card, role)?);
        }
    }
    Ok(rows)
}

pub(crate) fn skill_role_to_mcp(role: &SkillRole) -> Option<McpSurfaceRole> {
    match role {
        SkillRole::Tool => Some(McpSurfaceRole::Tool),
        SkillRole::Resource => Some(McpSurfaceRole::Resource),
        SkillRole::Prompt => Some(McpSurfaceRole::Prompt),
        SkillRole::Model => Some(McpSurfaceRole::Model),
        SkillRole::Memory | SkillRole::Retriever | SkillRole::Judge | SkillRole::Router => None,
    }
}

fn row_from_skill(card: &SkillCard, role: McpSurfaceRole) -> Result<McpSurfaceCard> {
    let name = stable_mcp_name_text(&card.id)?;
    let uri = (role == McpSurfaceRole::Resource).then(|| format!("skill://{}", card.id));
    Ok(McpSurfaceCard {
        id: format!("skill:{name}:{}", role.as_symbol()),
        source: McpSurfaceSource::SkillCard,
        role,
        name,
        symbol: Some(card.symbol.clone()),
        uri,
        description: card.description.clone(),
        input_shape: Some(card.input_shape.clone()),
        output_shape: Some(card.output_shape.clone()),
        annotations: Vec::new(),
        capabilities: card.capabilities.clone(),
        stream_policy: McpStreamPolicy::None,
    })
}
