use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};

use super::tool_schema::{
    argument_order, arguments_json, canonical_json, default_parameters, expr_text,
    expr_to_json_string, json_to_expr, json_to_value, validate_schema,
};

/// A single OpenAI function tool bound to a SIM callable.
///
/// Holds the OpenAI-facing function name and JSON Schema `parameters`
/// alongside the resolved SIM `symbol`, argument order, required
/// capabilities, and optional argument/result shapes used to translate
/// between OpenAI tool descriptors and SIM calls.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenAiTool {
    openai_name: String,
    symbol: Symbol,
    description: String,
    parameters: Value,
    arg_order: Vec<String>,
    capabilities: Vec<CapabilityName>,
    args_shape: Option<Expr>,
    result_shape: Option<Expr>,
}

/// Collection of [`OpenAiTool`] entries keyed by OpenAI tool name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenAiToolRegistry {
    tools: BTreeMap<String, OpenAiTool>,
}

/// A tool invocation requested by the model: an id, tool name, and JSON arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenAiToolCall {
    /// Provider-assigned call id correlating the call with its result.
    pub id: String,
    /// OpenAI tool name being invoked.
    pub name: String,
    /// Arguments supplied for the call as a JSON value.
    pub arguments: Value,
}

/// The outcome of an [`OpenAiToolCall`]: a status symbol and result payload.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenAiToolResult {
    /// Id of the originating [`OpenAiToolCall`].
    pub call_id: String,
    /// OpenAI tool name that produced this result.
    pub name: String,
    /// Outcome status, e.g. `ok`, `invalid-arguments`, or `unknown-tool`.
    pub status: Symbol,
    /// Result payload expression (the call output or an error message).
    pub output: Expr,
}

impl OpenAiTool {
    /// Builds a tool from a resolved SIM callable, capturing its argument and
    /// result shapes from the callable's browse metadata.
    pub fn from_callable(
        cx: &mut Cx,
        openai_name: impl Into<String>,
        symbol: Symbol,
        description: impl Into<String>,
        parameters: Value,
        capabilities: Vec<CapabilityName>,
    ) -> Result<Self> {
        let function = cx.resolve_function(&symbol)?;
        let Some(callable) = function.object().as_callable() else {
            return Err(Error::TypeMismatch {
                expected: "callable",
                found: "non-callable",
            });
        };
        let args_shape = callable
            .browse_args_shape(cx)?
            .map(|shape| shape.object().as_expr(cx))
            .transpose()?;
        let result_shape = callable
            .browse_result_shape(cx)?
            .map(|shape| shape.object().as_expr(cx))
            .transpose()?;
        Ok(Self::new(
            openai_name,
            symbol,
            description,
            parameters,
            capabilities,
            args_shape,
            result_shape,
        ))
    }

    /// Parses an OpenAI tool descriptor JSON object into an [`OpenAiTool`].
    ///
    /// Requires the descriptor `type` to be `function` and derives the SIM
    /// symbol from the function name. Inbound request JSON cannot set trusted
    /// SIM symbols or required capabilities; those come from the server-owned
    /// runtime registry.
    pub fn from_openai_descriptor(value: &Value) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| Error::Eval("openai tool descriptor must be an object".to_owned()))?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if kind != "function" {
            return Err(Error::Eval(format!(
                "unsupported OpenAI tool descriptor type {kind}"
            )));
        }
        let function = object
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Eval("openai tool missing function object".to_owned()))?;
        let openai_name = string_member(function, "name")?.to_owned();
        validate_openai_name(&openai_name)?;
        let description = function
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let parameters = function
            .get("parameters")
            .cloned()
            .unwrap_or_else(default_parameters);
        reject_untrusted_authority_fields(object)?;
        reject_untrusted_authority_fields(function)?;
        let symbol = openai_name_to_symbol(&openai_name);
        Ok(Self::new(
            openai_name,
            symbol,
            description,
            parameters,
            Vec::new(),
            None,
            None,
        ))
    }

    fn new(
        openai_name: impl Into<String>,
        symbol: Symbol,
        description: impl Into<String>,
        parameters: Value,
        capabilities: Vec<CapabilityName>,
        args_shape: Option<Expr>,
        result_shape: Option<Expr>,
    ) -> Self {
        let arg_order = argument_order(&parameters);
        Self {
            openai_name: openai_name.into(),
            symbol,
            description: description.into(),
            parameters,
            arg_order,
            capabilities,
            args_shape,
            result_shape,
        }
    }

    /// Returns the OpenAI-facing tool name.
    pub fn openai_name(&self) -> &str {
        &self.openai_name
    }

    /// Returns the SIM symbol this tool dispatches to.
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// Returns the capabilities required to invoke this tool.
    pub fn capabilities(&self) -> &[CapabilityName] {
        &self.capabilities
    }

    /// Validates `arguments` against the tool's JSON Schema parameters.
    pub fn validate_arguments(&self, arguments: &Value) -> std::result::Result<(), String> {
        validate_schema(&self.parameters, arguments, "arguments")
    }

    /// Validates and converts JSON `arguments` into ordered SIM values for the call.
    pub fn argument_values(
        &self,
        cx: &mut Cx,
        arguments: &Value,
    ) -> std::result::Result<Vec<sim_kernel::Value>, String> {
        self.validate_arguments(arguments)?;
        let Some(object) = arguments.as_object() else {
            return Err("tool arguments must be an object".to_owned());
        };
        self.arg_order
            .iter()
            .filter_map(|name| object.get(name))
            .map(|value| json_to_value(cx, value).map_err(|err| err.to_string()))
            .collect()
    }

    /// Renders this tool as an OpenAI tool descriptor JSON object, including
    /// the `x-sim-*` extension fields for symbol, shapes, and capabilities.
    pub fn descriptor_json(&self) -> Value {
        let mut function = Map::new();
        function.insert("name".to_owned(), Value::String(self.openai_name.clone()));
        function.insert(
            "description".to_owned(),
            Value::String(self.description.clone()),
        );
        function.insert("parameters".to_owned(), self.parameters.clone());
        function.insert(
            "x-sim-symbol".to_owned(),
            Value::String(self.symbol.as_qualified_str()),
        );
        if let Some(shape) = &self.args_shape {
            function.insert("x-sim-args-shape".to_owned(), expr_to_json_string(shape));
        }
        if let Some(shape) = &self.result_shape {
            function.insert("x-sim-result-shape".to_owned(), expr_to_json_string(shape));
        }
        if !self.capabilities.is_empty() {
            function.insert(
                "x-sim-capabilities".to_owned(),
                Value::Array(
                    self.capabilities
                        .iter()
                        .map(|capability| Value::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            );
        }
        let mut descriptor = Map::new();
        descriptor.insert("type".to_owned(), Value::String("function".to_owned()));
        descriptor.insert("function".to_owned(), Value::Object(function));
        Value::Object(descriptor)
    }

    /// Renders this tool as an OpenAI request descriptor without SIM authority
    /// extension fields.
    ///
    /// Use this form when a descriptor may cross an untrusted request boundary:
    /// the gateway resolves the SIM symbol and required capabilities from its
    /// server-owned registry rather than from `x-sim-*` request fields.
    pub fn request_descriptor_json(&self) -> Value {
        let mut function = Map::new();
        function.insert("name".to_owned(), Value::String(self.openai_name.clone()));
        function.insert(
            "description".to_owned(),
            Value::String(self.description.clone()),
        );
        function.insert("parameters".to_owned(), self.parameters.clone());
        let mut descriptor = Map::new();
        descriptor.insert("type".to_owned(), Value::String("function".to_owned()));
        descriptor.insert("function".to_owned(), Value::Object(function));
        Value::Object(descriptor)
    }

    /// Renders this tool as a SIM expression map.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::new("openai-gateway/tool"))),
            field("openai-name", Expr::String(self.openai_name.clone())),
            field("symbol", Expr::Symbol(self.symbol.clone())),
            field("description", Expr::String(self.description.clone())),
            field("parameters", json_to_expr(&self.parameters)),
            field(
                "capabilities",
                Expr::List(
                    self.capabilities
                        .iter()
                        .map(|capability| Expr::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            ),
            field("args-shape", self.args_shape.clone().unwrap_or(Expr::Nil)),
            field(
                "result-shape",
                self.result_shape.clone().unwrap_or(Expr::Nil),
            ),
        ])
    }
}

impl OpenAiToolRegistry {
    /// Builds a registry from the `tools` array of an OpenAI request object;
    /// a missing or null `tools` field yields an empty registry.
    pub fn from_request(cx: &mut Cx, object: &Map<String, Value>) -> Result<Self> {
        let Some(tools) = object.get("tools") else {
            return Ok(Self::default());
        };
        if tools.is_null() {
            return Ok(Self::default());
        }
        let tools = tools
            .as_array()
            .ok_or_else(|| Error::Eval("openai tools field must be an array".to_owned()))?;
        let mut registry = Self::default();
        for tool in tools {
            let tool = OpenAiTool::from_openai_descriptor(tool)?;
            ensure_server_registered_tool(cx, &tool)?;
            registry.insert(tool)?;
        }
        Ok(registry)
    }

    /// Inserts a tool, erroring if its OpenAI name is already registered.
    pub fn insert(&mut self, tool: OpenAiTool) -> Result<()> {
        if self.tools.contains_key(tool.openai_name()) {
            return Err(Error::Eval(format!(
                "duplicate OpenAI tool name {}",
                tool.openai_name()
            )));
        }
        self.tools.insert(tool.openai_name.clone(), tool);
        Ok(())
    }

    /// Returns `true` when no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Looks up a tool by its OpenAI name.
    pub fn get(&self, name: &str) -> Option<&OpenAiTool> {
        self.tools.get(name)
    }

    /// Renders the registry as a JSON array of OpenAI tool descriptors.
    pub fn descriptor_json(&self) -> Value {
        Value::Array(
            self.tools
                .values()
                .map(OpenAiTool::descriptor_json)
                .collect(),
        )
    }

    /// Renders the registry as a SIM list of tool expressions.
    pub fn to_expr(&self) -> Expr {
        Expr::List(self.tools.values().map(OpenAiTool::to_expr).collect())
    }
}

impl OpenAiToolCall {
    /// Extracts a tool call from a transcript content part, returning `None`
    /// when the part is not a `tool-call` map.
    pub fn from_content_part(part: &Expr) -> Result<Option<Self>> {
        let Expr::Map(entries) = part else {
            return Ok(None);
        };
        if symbol_field(entries, "type").as_deref() != Some("tool-call") {
            return Ok(None);
        }
        let name = required_string_field(entries, "name")?;
        let id = string_field(entries, "id").unwrap_or_else(|| format!("call_{name}"));
        let arguments = entries
            .iter()
            .find_map(|(key, value)| match key {
                Expr::Symbol(symbol)
                    if symbol.namespace.is_none() && symbol.name.as_ref() == "arguments" =>
                {
                    Some(arguments_json(value))
                }
                _ => None,
            })
            .transpose()?
            .unwrap_or_else(|| Value::Object(Map::new()));
        Ok(Some(Self {
            id,
            name,
            arguments,
        }))
    }

    /// Renders this tool call as a SIM expression map.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("id", Expr::String(self.id.clone())),
            field("name", Expr::String(self.name.clone())),
            field("arguments", json_to_expr(&self.arguments)),
        ])
    }

    /// Returns a stable `name:arguments` fingerprint for deduplicating calls.
    pub fn fingerprint(&self) -> String {
        format!("{}:{}", self.name, canonical_json(&self.arguments))
    }
}

impl OpenAiToolResult {
    /// Builds a successful (`ok`) result carrying `output` for `call`.
    pub fn success(call: &OpenAiToolCall, output: Expr) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            status: Symbol::new("ok"),
            output,
        }
    }

    /// Builds an `invalid-arguments` failure result for `call`.
    pub fn invalid_arguments(call: &OpenAiToolCall, message: impl Into<String>) -> Self {
        Self::failure(call, "invalid-arguments", message)
    }

    /// Builds a `capability-denied` failure result for `call`.
    pub fn capability_denied(call: &OpenAiToolCall, message: impl Into<String>) -> Self {
        Self::failure(call, "capability-denied", message)
    }

    /// Builds an `unknown-tool` failure result for `call`.
    pub fn unknown_tool(call: &OpenAiToolCall) -> Self {
        Self::failure(call, "unknown-tool", format!("unknown tool {}", call.name))
    }

    fn failure(call: &OpenAiToolCall, status: &'static str, message: impl Into<String>) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            status: Symbol::new(status),
            output: Expr::String(message.into()),
        }
    }

    /// Renders this tool result as a SIM expression map.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("tool-call-id", Expr::String(self.call_id.clone())),
            field("name", Expr::String(self.name.clone())),
            field("status", Expr::Symbol(self.status.clone())),
            field("output", self.output.clone()),
        ])
    }

    /// Returns a human-readable one-line summary of this result.
    pub fn message_text(&self) -> String {
        format!(
            "tool {} {}: {}",
            self.name,
            self.status.name,
            expr_text(&self.output)
        )
    }
}

/// Converts an OpenAI tool name into a SIM [`Symbol`], splitting on the first
/// `_` into a namespace and local name and replacing remaining `_` with `-`.
pub fn openai_name_to_symbol(name: &str) -> Symbol {
    if let Some((namespace, local)) = name.split_once('_') {
        Symbol::qualified(namespace.replace('_', "-"), local.replace('_', "-"))
    } else {
        Symbol::new(name.replace('_', "-"))
    }
}

/// Parses a SIM symbol from text, treating a `namespace/name` form as a
/// qualified symbol; errors on empty or malformed input.
pub fn symbol_from_text(text: &str) -> Result<Symbol> {
    if text.trim().is_empty() {
        return Err(Error::Eval("SIM tool symbol must not be empty".to_owned()));
    }
    Ok(if let Some((namespace, name)) = text.split_once('/') {
        if namespace.is_empty() || name.is_empty() {
            return Err(Error::Eval(format!("invalid SIM symbol {text}")));
        }
        Symbol::qualified(namespace.to_owned(), name.to_owned())
    } else {
        Symbol::new(text.to_owned())
    })
}

fn validate_openai_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Eval("OpenAI tool name must not be empty".to_owned()));
    }
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(())
    } else {
        Err(Error::Eval(format!(
            "OpenAI tool name {name} contains unsupported characters"
        )))
    }
}

fn string_member<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Eval(format!("openai tool missing string {name}")))
}

fn ensure_server_registered_tool(cx: &mut Cx, tool: &OpenAiTool) -> Result<()> {
    let function = cx.resolve_function(tool.symbol())?;
    if function.object().as_callable().is_some() {
        Ok(())
    } else {
        Err(Error::TypeMismatch {
            expected: "callable",
            found: "non-callable",
        })
    }
}

fn reject_untrusted_authority_fields(object: &Map<String, Value>) -> Result<()> {
    for field in ["x-sim-symbol", "x-sim-capabilities"] {
        if object.contains_key(field) {
            return Err(Error::Eval(format!(
                "untrusted OpenAI tool descriptor cannot set {field}"
            )));
        }
    }
    Ok(())
}

fn required_string_field(entries: &[(Expr, Expr)], name: &str) -> Result<String> {
    string_field(entries, name)
        .ok_or_else(|| Error::Eval(format!("tool-call missing string {name}")))
}

fn string_field(entries: &[(Expr, Expr)], name: &str) -> Option<String> {
    sim_value::access::entry_field(entries, name)
        .and_then(sim_value::access::as_str)
        .map(str::to_owned)
}

fn symbol_field(entries: &[(Expr, Expr)], name: &str) -> Option<String> {
    match sim_value::access::entry_field(entries, name) {
        Some(Expr::Symbol(value)) => Some(value.name.as_ref().to_owned()),
        _ => None,
    }
}

use sim_value::build::entry as field;
