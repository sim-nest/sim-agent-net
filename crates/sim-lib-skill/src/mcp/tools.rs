use std::sync::Arc;

use sim_citizen_derive::Citizen;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_shape::{AnyShape, parse_shape_expr, shape_value};

use crate::{SkillCard, SkillPolicy, SkillRole, skill_specific_call_capability};

/// MCP `tools/list` descriptor for a skill exposed as a tool.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "skill/McpToolDescriptor", version = 1)]
pub struct McpToolDescriptor {
    /// Tool name (the skill id).
    pub name: String,
    /// Optional human-readable title.
    pub title: Option<String>,
    /// Description of what the tool does.
    pub description: String,
    /// Input schema expression for the tool's arguments.
    pub input_schema: Expr,
    /// Optional output schema expression for the tool's result.
    pub output_schema: Option<Expr>,
}

/// Parameters for an MCP `tools/call` request.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "skill/McpCallParams", version = 1)]
pub struct McpCallParams {
    /// Name of the tool to call.
    pub name: String,
    /// Positional argument expressions for the call.
    pub arguments: Vec<Expr>,
}

/// Result of an MCP `tools/call` request.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "skill/McpToolResult", version = 1)]
pub struct McpToolResult {
    /// Result expression returned by the tool.
    pub result: Expr,
    /// Whether the result represents an error.
    pub is_error: bool,
}

impl Default for McpToolDescriptor {
    fn default() -> Self {
        Self {
            name: "citizen-tool".to_owned(),
            title: Some("Citizen Tool".to_owned()),
            description: "Citizen fixture tool descriptor".to_owned(),
            input_schema: Expr::Symbol(Symbol::new("Any")),
            output_schema: Some(Expr::Symbol(Symbol::new("Any"))),
        }
    }
}

impl Default for McpCallParams {
    fn default() -> Self {
        Self {
            name: "citizen-tool".to_owned(),
            arguments: vec![Expr::String("fixture".to_owned())],
        }
    }
}

impl Default for McpToolResult {
    fn default() -> Self {
        Self {
            result: Expr::String("ok".to_owned()),
            is_error: false,
        }
    }
}

impl McpToolDescriptor {
    /// Builds a descriptor from a skill `card`, deriving the schemas from its
    /// input and output shapes.
    pub fn from_skill_card(cx: &mut Cx, card: &SkillCard) -> Result<Self> {
        Ok(Self {
            name: card.id.clone(),
            title: Some(card.title.clone()),
            description: card.description.clone(),
            input_schema: parseable_shape_expr(cx, &card.input_shape)?,
            output_schema: Some(parseable_shape_expr(cx, &card.output_shape)?),
        })
    }

    /// Builds a skill card from this descriptor, bound to `transport_id`.
    pub fn to_skill_card(&self, transport_id: impl Into<String>) -> Result<SkillCard> {
        let transport_id = transport_id.into();
        let input_shape = parse_shape_expr(&self.input_schema)?;
        let output_shape = match &self.output_schema {
            Some(expr) => parse_shape_expr(expr)?,
            None => Arc::new(AnyShape),
        };
        Ok(SkillCard {
            id: self.name.clone(),
            symbol: Symbol::qualified("skill", self.name.clone()),
            aliases: Vec::new(),
            origin: Symbol::new("mcp-tool"),
            title: self.title.clone().unwrap_or_else(|| self.name.clone()),
            description: self.description.clone(),
            input_shape: shape_value(
                Symbol::qualified(format!("skill/{}", self.name), "args"),
                input_shape,
            ),
            output_shape: shape_value(
                Symbol::qualified(format!("skill/{}", self.name), "result"),
                output_shape,
            ),
            roles: vec![SkillRole::Tool],
            capabilities: vec![skill_specific_call_capability(&self.name)],
            policy: SkillPolicy::default(),
            transport_id,
            transport_kind: "mcp".to_owned(),
            operation: self.name.clone(),
        })
    }

    /// Encodes the descriptor as an MCP `mcp/tool` map expression.
    pub fn to_expr(&self) -> Expr {
        let mut fields = vec![
            field("kind", Expr::Symbol(Symbol::qualified("mcp", "tool"))),
            field("name", Expr::String(self.name.clone())),
            field("description", Expr::String(self.description.clone())),
            field("inputSchema", self.input_schema.clone()),
        ];
        fields.push(field(
            "title",
            self.title
                .as_ref()
                .map(|title| Expr::String(title.clone()))
                .unwrap_or(Expr::Nil),
        ));
        fields.push(field(
            "outputSchema",
            self.output_schema.clone().unwrap_or(Expr::Nil),
        ));
        Expr::Map(fields)
    }

    /// Decodes a descriptor from an MCP tool map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP tool descriptor")?;
        Ok(Self {
            name: required_string(fields, "name")?,
            title: optional_string(fields, "title")?,
            description: required_string(fields, "description")?,
            input_schema: required_field_any(fields, &["inputSchema", "input-schema"])?.clone(),
            output_schema: optional_field_any(fields, &["outputSchema", "output-schema"]).cloned(),
        })
    }
}

impl McpCallParams {
    /// Encodes the parameters as an MCP `mcp/tools-call` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::qualified("mcp", "tools-call"))),
            field("name", Expr::String(self.name.clone())),
            field("arguments", Expr::List(self.arguments.clone())),
        ])
    }

    /// Decodes the parameters from an MCP tools/call map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP tools/call params")?;
        Ok(Self {
            name: required_string(fields, "name")?,
            arguments: match required_field(fields, "arguments")? {
                Expr::List(items) => items.clone(),
                _ => {
                    return Err(Error::TypeMismatch {
                        expected: "argument list",
                        found: "non-list",
                    });
                }
            },
        })
    }

    /// Decodes the parameters from a runtime `value`.
    pub fn from_value(cx: &mut Cx, value: &Value) -> Result<Self> {
        Self::from_expr(&value.object().as_expr(cx)?)
    }
}

impl McpToolResult {
    /// Builds a successful result wrapping `value`.
    pub fn success(cx: &mut Cx, value: Value) -> Result<Self> {
        Ok(Self {
            result: value.object().as_expr(cx)?,
            is_error: false,
        })
    }

    /// Encodes the result as an MCP `mcp/tool-result` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field(
                "kind",
                Expr::Symbol(Symbol::qualified("mcp", "tool-result")),
            ),
            field("result", self.result.clone()),
            field("isError", Expr::Bool(self.is_error)),
        ])
    }

    /// Decodes a result from an MCP tool-result map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP tool result")?;
        Ok(Self {
            result: required_field(fields, "result")?.clone(),
            is_error: match required_field_any(fields, &["isError", "is-error"])? {
                Expr::Bool(value) => *value,
                _ => {
                    return Err(Error::TypeMismatch {
                        expected: "bool",
                        found: "non-bool",
                    });
                }
            },
        })
    }

    /// Encodes the result as a runtime [`Value`].
    pub fn to_value(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().expr(self.to_expr())
    }
}

use sim_value::access::map_entries as map_fields;

fn required_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr> {
    required_field_any(fields, &[name])
}

fn required_field_any<'a>(fields: &'a [(Expr, Expr)], names: &[&str]) -> Result<&'a Expr> {
    optional_field_any(fields, names)
        .ok_or_else(|| Error::Eval(format!("MCP tool descriptor is missing {}", names[0])))
}

fn optional_field_any<'a>(fields: &'a [(Expr, Expr)], names: &[&str]) -> Option<&'a Expr> {
    fields.iter().find_map(|(key, value)| {
        let name = field_name(key)?;
        names.contains(&name.as_str()).then_some(value)
    })
}

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
    match optional_field_any(fields, &[name]) {
        Some(Expr::String(value)) => Ok(Some(value.clone())),
        Some(Expr::Nil) | None => Ok(None),
        Some(_) => Err(Error::TypeMismatch {
            expected: "string or nil",
            found: "invalid optional string",
        }),
    }
}

fn field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.to_string()),
        Expr::String(value) => Some(value.clone()),
        _ => None,
    }
}

use sim_value::build::entry as field;

fn parseable_shape_expr(cx: &mut Cx, shape: &Value) -> Result<Expr> {
    let expr = shape.object().as_expr(cx)?;
    if parse_shape_expr(&expr).is_ok() {
        Ok(expr)
    } else {
        Ok(Expr::Symbol(Symbol::new("Any")))
    }
}

/// Returns the class symbol for [`McpToolDescriptor`].
pub fn mcp_tool_descriptor_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpToolDescriptor")
}

/// Returns the class symbol for [`McpCallParams`].
pub fn mcp_call_params_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpCallParams")
}

/// Returns the class symbol for [`McpToolResult`].
pub fn mcp_tool_result_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpToolResult")
}
