use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// MCP `prompts/list` descriptor for a prompt-role skill.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "skill/McpPromptDescriptor", version = 1)]
pub struct McpPromptDescriptor {
    /// Prompt name.
    pub name: String,
    /// Description of the prompt.
    pub description: String,
    /// Declared prompt arguments.
    pub arguments: Vec<McpPromptArgument>,
}

/// One declared argument of an MCP prompt.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "skill/McpPromptArgument", version = 1)]
pub struct McpPromptArgument {
    /// Argument name.
    pub name: String,
    /// Description of the argument.
    pub description: String,
    /// Whether the argument is required.
    pub required: bool,
}

/// Parameters for an MCP `prompts/get` request.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "skill/McpPromptGetParams", version = 1)]
pub struct McpPromptGetParams {
    /// Name of the prompt to get.
    pub name: String,
    /// Name/value argument pairs supplied to the prompt.
    pub arguments: Vec<(String, String)>,
}

impl Default for McpPromptDescriptor {
    fn default() -> Self {
        Self {
            name: "citizen-prompt".to_owned(),
            description: "Citizen fixture prompt".to_owned(),
            arguments: vec![McpPromptArgument::default()],
        }
    }
}

impl Default for McpPromptArgument {
    fn default() -> Self {
        Self {
            name: "topic".to_owned(),
            description: "Prompt topic".to_owned(),
            required: true,
        }
    }
}

impl Default for McpPromptGetParams {
    fn default() -> Self {
        Self {
            name: "citizen-prompt".to_owned(),
            arguments: vec![("topic".to_owned(), "fixtures".to_owned())],
        }
    }
}

impl McpPromptDescriptor {
    /// Encodes the descriptor as an MCP `mcp/prompt` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::qualified("mcp", "prompt"))),
            field("name", Expr::String(self.name.clone())),
            field("description", Expr::String(self.description.clone())),
            field(
                "arguments",
                Expr::List(
                    self.arguments
                        .iter()
                        .map(McpPromptArgument::to_expr)
                        .collect(),
                ),
            ),
        ])
    }

    /// Decodes a descriptor from an MCP prompt map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP prompt descriptor")?;
        let arguments = match required_field(fields, "arguments")? {
            Expr::List(items) => items
                .iter()
                .map(McpPromptArgument::from_expr)
                .collect::<Result<Vec<_>>>()?,
            _ => {
                return Err(Error::TypeMismatch {
                    expected: "argument list",
                    found: "non-list",
                });
            }
        };
        Ok(Self {
            name: required_string(fields, "name")?,
            description: required_string(fields, "description")?,
            arguments,
        })
    }
}

impl McpPromptArgument {
    /// Encodes the argument as a map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("name", Expr::String(self.name.clone())),
            field("description", Expr::String(self.description.clone())),
            field("required", Expr::Bool(self.required)),
        ])
    }

    /// Decodes an argument from a map expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "MCP prompt argument")?;
        Ok(Self {
            name: required_string(fields, "name")?,
            description: required_string(fields, "description")?,
            required: required_bool(fields, "required")?,
        })
    }
}

impl McpPromptGetParams {
    /// Encodes the parameters as an MCP `mcp/prompts-get` map expression.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field(
                "kind",
                Expr::Symbol(Symbol::qualified("mcp", "prompts-get")),
            ),
            field("name", Expr::String(self.name.clone())),
            field(
                "arguments",
                Expr::Map(
                    self.arguments
                        .iter()
                        .map(|(key, value)| {
                            (Expr::String(key.clone()), Expr::String(value.clone()))
                        })
                        .collect(),
                ),
            ),
        ])
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

fn required_bool(fields: &[(Expr, Expr)], name: &str) -> Result<bool> {
    match required_field(fields, name)? {
        Expr::Bool(value) => Ok(*value),
        _ => Err(Error::TypeMismatch {
            expected: "bool",
            found: "non-bool",
        }),
    }
}

fn required_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr> {
    sim_value::access::entry_field(fields, name)
        .ok_or_else(|| Error::Eval(format!("MCP prompt descriptor is missing {name}")))
}

use sim_value::build::entry as field;

impl CitizenField for McpPromptArgument {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.name.encode_field(),
            self.description.encode_field(),
            self.required.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(items) = expr else {
            return Err(sim_citizen::field_error(
                field,
                "expected MCP prompt argument list",
            ));
        };
        let [name, description, required] = items.as_slice() else {
            return Err(sim_citizen::field_error(
                field,
                format!(
                    "expected 3 MCP prompt argument field(s), found {}",
                    items.len()
                ),
            ));
        };
        Ok(Self {
            name: String::decode_field_expr(name, field)?,
            description: String::decode_field_expr(description, field)?,
            required: bool::decode_field_expr(required, field)?,
        })
    }
}

/// Returns the class symbol for [`McpPromptDescriptor`].
pub fn mcp_prompt_descriptor_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpPromptDescriptor")
}

/// Returns the class symbol for [`McpPromptArgument`].
pub fn mcp_prompt_argument_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpPromptArgument")
}

/// Returns the class symbol for [`McpPromptGetParams`].
pub fn mcp_prompt_get_params_class_symbol() -> Symbol {
    Symbol::qualified("skill", "McpPromptGetParams")
}
