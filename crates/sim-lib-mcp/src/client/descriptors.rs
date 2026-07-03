use std::sync::Arc;

use sim_kernel::{Error, Expr, Result, Symbol, Value};
use sim_lib_skill::{SkillCard, SkillRole};
use sim_shape::{AnyShape, shape_value};

use crate::surface::stable_mcp_name_text;

use super::optional_field_from_fields;

pub(crate) struct ForeignToolDescriptor {
    pub(crate) name: String,
    title: Option<String>,
    description: String,
}

pub(crate) struct ForeignResourceDescriptor {
    pub(crate) uri: String,
    name: String,
    description: String,
}

pub(crate) struct ForeignPromptDescriptor {
    pub(crate) name: String,
    description: String,
}

impl ForeignToolDescriptor {
    pub(crate) fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "foreign MCP tool descriptor")?;
        let name = required_string(fields, "name")?;
        Ok(Self {
            name: name.clone(),
            title: optional_string(fields, "title")?,
            description: optional_string(fields, "description")?
                .unwrap_or_else(|| format!("Foreign MCP tool {name}")),
        })
    }

    pub(crate) fn to_skill_card(&self, client_id: &str, transport_id: &str) -> Result<SkillCard> {
        skill_card(
            imported_id(client_id, "tool", &self.name)?,
            self.title.clone().unwrap_or_else(|| self.name.clone()),
            self.description.clone(),
            SkillRole::Tool,
            transport_id,
            operation_id("tool", &self.name)?,
        )
    }
}

impl ForeignResourceDescriptor {
    pub(crate) fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "foreign MCP resource descriptor")?;
        let uri = required_string(fields, "uri")?;
        Ok(Self {
            uri: uri.clone(),
            name: optional_string(fields, "name")?.unwrap_or_else(|| uri.clone()),
            description: optional_string(fields, "description")?
                .unwrap_or_else(|| format!("Foreign MCP resource {uri}")),
        })
    }

    pub(crate) fn to_skill_card(&self, client_id: &str, transport_id: &str) -> Result<SkillCard> {
        skill_card(
            imported_id(client_id, "resource", &self.uri)?,
            self.name.clone(),
            self.description.clone(),
            SkillRole::Resource,
            transport_id,
            operation_id("resource", &self.uri)?,
        )
    }
}

impl ForeignPromptDescriptor {
    pub(crate) fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "foreign MCP prompt descriptor")?;
        let name = required_string(fields, "name")?;
        Ok(Self {
            name: name.clone(),
            description: optional_string(fields, "description")?
                .unwrap_or_else(|| format!("Foreign MCP prompt {name}")),
        })
    }

    pub(crate) fn to_skill_card(&self, client_id: &str, transport_id: &str) -> Result<SkillCard> {
        skill_card(
            imported_id(client_id, "prompt", &self.name)?,
            self.name.clone(),
            self.description.clone(),
            SkillRole::Prompt,
            transport_id,
            operation_id("prompt", &self.name)?,
        )
    }
}

fn skill_card(
    id: String,
    title: String,
    description: String,
    role: SkillRole,
    transport_id: &str,
    operation: String,
) -> Result<SkillCard> {
    Ok(SkillCard {
        symbol: Symbol::qualified("skill", id.clone()),
        aliases: Vec::new(),
        origin: Symbol::qualified("mcp-client", transport_id.to_owned()),
        title,
        description,
        input_shape: any_shape(&id, "args"),
        output_shape: any_shape(&id, "result"),
        roles: vec![role],
        capabilities: vec![sim_lib_skill::skill_specific_call_capability(&id)],
        policy: sim_lib_skill::SkillPolicy::default(),
        transport_id: transport_id.to_owned(),
        transport_kind: "mcp-client".to_owned(),
        operation,
        id,
    })
}

fn imported_id(client_id: &str, role: &str, name: &str) -> Result<String> {
    Ok(format!(
        "{}.{}.{}",
        stable_mcp_name_text(client_id)?,
        role,
        stable_mcp_name_text(name)?
    ))
}

fn operation_id(role: &str, name: &str) -> Result<String> {
    Ok(format!("{role}:{}", stable_mcp_name_text(name)?))
}

fn any_shape(id: &str, suffix: &str) -> Value {
    shape_value(
        Symbol::qualified(format!("mcp-client/{id}"), suffix.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_value::access::map_entries as map_fields;

fn required_string(fields: &[(Expr, Expr)], name: &str) -> Result<String> {
    match optional_field_from_fields(fields, name) {
        Some(Expr::String(value)) => Ok(value.clone()),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
        None => Err(Error::Eval(format!(
            "foreign MCP descriptor missing {name}"
        ))),
    }
}

fn optional_string(fields: &[(Expr, Expr)], name: &str) -> Result<Option<String>> {
    match optional_field_from_fields(fields, name) {
        Some(Expr::String(value)) => Ok(Some(value.clone())),
        Some(Expr::Nil) | None => Ok(None),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string or nil",
            found: "invalid optional string",
        }),
    }
}
