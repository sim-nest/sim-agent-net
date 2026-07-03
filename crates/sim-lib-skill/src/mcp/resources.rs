use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// MCP `resources/list` descriptor for a resource-role skill.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "skill/McpResourceDescriptor", version = 1)]
pub struct McpResourceDescriptor {
    /// Resource URI.
    pub uri: String,
    /// Resource name.
    pub name: String,
    /// Description of the resource.
    pub description: String,
    /// Optional MIME type of the resource contents.
    pub mime_type: Option<String>,
}

/// Parameters for an MCP `resources/read` request.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "skill/McpResourceReadParams", version = 1)]
pub struct McpResourceReadParams {
    /// URI of the resource to read.
    pub uri: String,
}

impl Default for McpResourceDescriptor {
    fn default() -> Self {
        Self {
            uri: "sim://citizen/resource".to_owned(),
            name: "citizen-resource".to_owned(),
            description: "Citizen fixture resource".to_owned(),
            mime_type: Some("text/plain".to_owned()),
        }
    }
}

impl Default for McpResourceReadParams {
    fn default() -> Self {
        Self {
            uri: "sim://citizen/resource".to_owned(),
        }
    }
}

impl McpResourceDescriptor {
    /// Encodes the descriptor as an MCP `mcp/resource` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::qualified("mcp", "resource"))),
            field("uri", Expr::String(self.uri.clone())),
            field("name", Expr::String(self.name.clone())),
            field("description", Expr::String(self.description.clone())),
            field(
                "mimeType",
                self.mime_type
                    .as_ref()
                    .map(|value| Expr::String(value.clone()))
                    .unwrap_or(Expr::Nil),
            ),
        ])
    }

    /// Decodes a descriptor from an MCP resource map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP resource descriptor")?;
        Ok(Self {
            uri: required_string(fields, "uri")?,
            name: required_string(fields, "name")?,
            description: required_string(fields, "description")?,
            mime_type: optional_string(fields, "mimeType")?,
        })
    }
}

impl McpResourceReadParams {
    /// Encodes the parameters as an MCP `mcp/resources-read` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field(
                "kind",
                Expr::Symbol(Symbol::qualified("mcp", "resources-read")),
            ),
            field("uri", Expr::String(self.uri.clone())),
        ])
    }

    /// Decodes the parameters from an MCP resources/read map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP resources/read params")?;
        Ok(Self {
            uri: required_string(fields, "uri")?,
        })
    }
}

use sim_value::access::map_entries as map_fields;

fn required_string(fields: &[(Expr, Expr)], name: &str) -> Result<String> {
    match required_field(fields, name)? {
        Expr::String(value) => Ok(value.clone()),
        _ => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
    }
}

fn optional_string(fields: &[(Expr, Expr)], name: &str) -> Result<Option<String>> {
    match optional_field(fields, name) {
        Some(Expr::String(value)) => Ok(Some(value.clone())),
        Some(Expr::Nil) | None => Ok(None),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string or nil",
            found: "invalid optional string",
        }),
    }
}

fn required_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr> {
    optional_field(fields, name)
        .ok_or_else(|| Error::Eval(format!("MCP resource descriptor is missing {name}")))
}

use sim_value::access::entry_field as optional_field;

use sim_value::build::entry as field;

/// Returns the class symbol for [`McpResourceDescriptor`].
pub fn mcp_resource_descriptor_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpResourceDescriptor")
}

/// Returns the class symbol for [`McpResourceReadParams`].
pub fn mcp_resource_read_params_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpResourceReadParams")
}
