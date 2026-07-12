use crate::util::{
    installed_codecs, keyword, literal_expr, parse_capabilities_expr, shape_from_expr, shape_id,
    string_literal, symbol_from_eval, symbol_from_value, symbol_of, value_from_expr,
};
use crate::{AGENT_LIB_ID, Component, ComponentKind, TOOL_EXPORT_KIND};
use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, CapabilityName, ClassRef, Cx, Error, EvalReply,
    ExportKind, ExportRecord, ExportState, Expr, Object, Result, ShapeRef, Symbol, Value,
};
use sim_lib_server::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_reply,
};
use sim_shape::{ShapeReport, check_value_report};
use std::{any::Any, sync::Arc};

#[derive(Clone)]
/// A callable tool an agent can invoke, with shape-checked arguments and
/// results, a category, and required capabilities.
pub struct Tool {
    /// The tool's name.
    pub symbol: Symbol,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// Shape the argument list is checked against before invocation.
    pub args_shape: ShapeRef,
    /// Optional shape the result is checked against after invocation.
    pub result_shape: Option<ShapeRef>,
    /// Category grouping this tool with related tools.
    pub category: Symbol,
    /// Capabilities required to invoke the tool.
    pub capabilities: Vec<CapabilityName>,
    /// The underlying callable that implements the tool.
    pub function: Value,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
}

impl Tool {
    fn args_match(&self, cx: &mut Cx, args: &[Value]) -> Result<ShapeReport> {
        let args_value = cx.factory().list(args.to_vec())?;
        check_value_report(cx, &self.args_shape, args_value)
    }

    fn check_result_shape(&self, cx: &mut Cx, value: Value) -> Result<Value> {
        let Some(result_shape) = &self.result_shape else {
            return Ok(value);
        };
        let matched = check_value_report(cx, result_shape, value.clone())?;
        if matched.accepted {
            Ok(value)
        } else {
            Err(Error::WrongShape {
                expected: shape_id(result_shape),
                diagnostics: matched.diagnostics,
            })
        }
    }

    pub(crate) fn call_values(&self, cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
        for capability in &self.capabilities {
            cx.require(capability)?;
        }
        let matched = self.args_match(cx, &args)?;
        if !matched.accepted {
            return Err(Error::WrongShape {
                expected: shape_id(&self.args_shape),
                diagnostics: matched.diagnostics,
            });
        }
        let value = cx.call_value(self.function.clone(), Args::new(args))?;
        self.check_result_shape(cx, value)
    }

    pub(crate) fn metadata_entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let capabilities = cx.factory().list(
            self.capabilities
                .iter()
                .map(|capability| cx.factory().string(capability.as_str().to_owned()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let result_shape = match &self.result_shape {
            Some(shape) => shape.clone(),
            None => cx.factory().nil()?,
        };
        Ok(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(tool_export_kind_symbol())?,
            ),
            (
                Symbol::new("name"),
                cx.factory().symbol(self.symbol.clone())?,
            ),
            (
                Symbol::new("description"),
                cx.factory().string(self.description.clone())?,
            ),
            (Symbol::new("args"), self.args_shape.clone()),
            (Symbol::new("result"), result_shape),
            (
                Symbol::new("category"),
                cx.factory().symbol(self.category.clone())?,
            ),
            (Symbol::new("capabilities"), capabilities),
        ])
    }

    pub(crate) fn model_descriptor_expr(&self, cx: &mut Cx) -> Result<Expr> {
        let args = self.args_shape.object().as_expr(cx)?;
        let result = self
            .result_shape
            .as_ref()
            .map(|shape| shape.object().as_expr(cx))
            .transpose()?
            .unwrap_or(Expr::Nil);
        let capabilities = self
            .capabilities
            .iter()
            .map(|capability| Expr::String(capability.as_str().to_owned()))
            .collect();
        Ok(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("name")),
                Expr::Symbol(self.symbol.clone()),
            ),
            (
                Expr::Symbol(Symbol::new("description")),
                Expr::String(self.description.clone()),
            ),
            (Expr::Symbol(Symbol::new("args")), args),
            (Expr::Symbol(Symbol::new("result")), result),
            (
                Expr::Symbol(Symbol::new("category")),
                Expr::Symbol(self.category.clone()),
            ),
            (
                Expr::Symbol(Symbol::new("capabilities")),
                Expr::List(capabilities),
            ),
        ]))
    }

    fn metadata_value(&self, cx: &mut Cx) -> Result<Value> {
        let entries = self.metadata_entries(cx)?;
        cx.factory().table(entries)
    }
}

impl Object for Tool {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<tool {}>", self.symbol))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for Tool {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        self.metadata_value(cx)
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for Tool {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.call_values(cx, args.into_vec())
    }
}

impl EvalSite for Tool {
    fn site_kind(&self) -> &'static str {
        TOOL_EXPORT_KIND
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if frame.kind != FrameKind::Request {
            return Err(Error::Eval(format!(
                "tool {} cannot answer frame kind {}",
                self.symbol,
                frame.kind.as_symbol()
            )));
        }
        let consistency = frame.envelope.consistency;
        let request = eval_request_from_frame(cx, &frame)?;
        let args = expr_args_to_values(cx, request.expr)?;
        let value = self.call_values(cx, args)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Component for Tool {
    fn kind(&self) -> ComponentKind {
        ComponentKind::Tool
    }

    fn name(&self) -> &Symbol {
        &self.symbol
    }

    fn capabilities(&self) -> &[CapabilityName] {
        &self.capabilities
    }

    fn reflect(&self, cx: &mut Cx) -> Result<Expr> {
        self.metadata_value(cx)?.object().as_expr(cx)
    }
}

#[derive(Default)]
pub(crate) struct ToolFilter {
    pub(crate) category: Option<Symbol>,
    pub(crate) agent: Option<Symbol>,
}

pub(crate) fn define_tool(cx: &mut Cx, exprs: Vec<Expr>) -> Result<Value> {
    let Some((name_expr, rest)) = exprs.split_first() else {
        return Err(Error::Eval(
            "agent/defun expects a name, metadata options, and a callable body".to_owned(),
        ));
    };
    let name = symbol_of(name_expr, "agent/defun expects a symbol name")?;
    if rest.len() < 2 {
        return Err(Error::Eval(
            "agent/defun expects metadata options followed by a callable body".to_owned(),
        ));
    }
    let (body_expr, option_exprs) = rest
        .split_last()
        .ok_or_else(|| Error::Eval("agent/defun requires a callable body".to_owned()))?;
    if !option_exprs.len().is_multiple_of(2) {
        return Err(Error::Eval(
            "agent/defun options must be key/value pairs".to_owned(),
        ));
    }

    let mut description = None;
    let mut args_shape = None;
    let mut result_shape = None;
    let mut category = None;
    let mut capabilities = Vec::new();

    for pair in option_exprs.chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "description" => {
                description = Some(string_literal(
                    &pair[1],
                    "agent/defun :description expects a string",
                )?);
            }
            "args" => {
                args_shape = Some(shape_from_expr(cx, &pair[1], &name, "args")?);
            }
            "result" => {
                result_shape = Some(shape_from_expr(cx, &pair[1], &name, "result")?);
            }
            "category" => {
                category = Some(symbol_of(
                    literal_expr(&pair[1]),
                    "agent/defun :category expects a symbol",
                )?);
            }
            "capable" => {
                capabilities = parse_capabilities_expr(literal_expr(&pair[1]))?;
            }
            other => return Err(Error::Eval(format!("agent/defun: unknown option :{other}"))),
        }
    }

    let function = cx.eval_expr(body_expr.clone())?;
    if function.object().as_callable().is_none() {
        return Err(Error::TypeMismatch {
            expected: "callable",
            found: "non-callable",
        });
    }

    let tool = Arc::new(Tool {
        symbol: name.clone(),
        description: description
            .ok_or_else(|| Error::Eval("agent/defun requires :description".to_owned()))?,
        args_shape: args_shape
            .ok_or_else(|| Error::Eval("agent/defun requires :args".to_owned()))?,
        result_shape,
        category: category.unwrap_or_else(|| Symbol::new("general")),
        capabilities,
        function,
        address: ServerAddress::Local,
        codecs: installed_codecs(cx),
    });
    let value = cx.factory().opaque(tool.clone())?;
    register_tool(cx, tool, value.clone())?;
    Ok(value)
}

pub(crate) fn parse_tools_exprs(cx: &mut Cx, exprs: Vec<Expr>) -> Result<Value> {
    if exprs.is_empty() {
        return list_tools_value(cx, ToolFilter::default());
    }
    if !exprs.len().is_multiple_of(2) {
        return Err(Error::Eval(
            "agent/tools options must be key/value pairs".to_owned(),
        ));
    }
    let mut filter = ToolFilter::default();
    for pair in exprs.chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "category" => {
                filter.category = Some(symbol_from_eval(
                    cx,
                    &pair[1],
                    "agent/tools :category expects a symbol",
                )?);
            }
            "agent" => {
                filter.agent = Some(symbol_from_eval(
                    cx,
                    &pair[1],
                    "agent/tools :agent expects a symbol",
                )?);
            }
            other => return Err(Error::Eval(format!("agent/tools: unknown option :{other}"))),
        }
    }
    list_tools_value(cx, filter)
}

pub(crate) fn call_tool_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let Some(target) = args.values().first() else {
        return Err(Error::Eval(
            "agent/call-tool expects a tool name or tool value".to_owned(),
        ));
    };
    let tool_value = resolve_tool_value(cx, target.clone())?;
    let tool = tool_value
        .object()
        .downcast_ref::<Tool>()
        .ok_or(Error::TypeMismatch {
            expected: "tool",
            found: "non-tool",
        })?;
    tool.call_values(cx, args.values()[1..].to_vec())
}

pub(crate) fn list_tools_value(cx: &mut Cx, filter: ToolFilter) -> Result<Value> {
    let tools = registered_tools(cx)?
        .into_iter()
        .filter(|tool| {
            filter
                .category
                .as_ref()
                .is_none_or(|category| &tool.category == category)
        })
        .filter(|_| filter.agent.is_none())
        .map(|tool| tool.metadata_value(cx))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(tools)
}

pub(crate) fn register_tool(cx: &mut Cx, tool: Arc<Tool>, value: Value) -> Result<()> {
    let lib = Symbol::new(AGENT_LIB_ID);
    cx.registry_mut()
        .register_value_for_lib(&lib, tool.symbol.clone(), value)?;
    cx.registry_mut().append_export_record(
        &lib,
        ExportRecord {
            kind: tool_export_kind(),
            symbol: tool.symbol.clone(),
            state: ExportState::Resolved {
                id: sim_kernel::RuntimeId::Value,
            },
        },
    )?;
    Ok(())
}

pub(crate) fn resolve_tool_by_symbol(cx: &mut Cx, symbol: &Symbol) -> Result<Tool> {
    let tool_value = cx.resolve_value(symbol)?;
    tool_value
        .object()
        .downcast_ref::<Tool>()
        .cloned()
        .ok_or(Error::TypeMismatch {
            expected: "tool",
            found: "non-tool",
        })
}

fn resolve_tool_value(cx: &mut Cx, value: Value) -> Result<Value> {
    if value.object().downcast_ref::<Tool>().is_some() {
        return Ok(value);
    }
    let symbol = symbol_from_value(
        cx,
        value,
        "agent/call-tool expects a tool symbol or tool value",
    )?;
    let tool_value = cx.resolve_value(&symbol)?;
    if tool_value.object().downcast_ref::<Tool>().is_some() {
        Ok(tool_value)
    } else {
        Err(Error::TypeMismatch {
            expected: "tool",
            found: "non-tool",
        })
    }
}

fn registered_tools(cx: &Cx) -> Result<Vec<Arc<Tool>>> {
    let loaded = cx
        .registry()
        .lib(&Symbol::new(AGENT_LIB_ID))
        .ok_or_else(|| Error::Lib("agent lib is not installed".to_owned()))?;
    let tools = loaded
        .exports
        .iter()
        .filter(|record| record.kind == tool_export_kind())
        .filter_map(|record| cx.registry().value_by_symbol(&record.symbol))
        .filter_map(|value| value.object().downcast_ref::<Tool>())
        .cloned()
        .map(Arc::new)
        .collect();
    Ok(tools)
}

fn expr_args_to_values(cx: &mut Cx, expr: Expr) -> Result<Vec<Value>> {
    match expr {
        Expr::List(items) | Expr::Vector(items) => items
            .iter()
            .map(|item| value_from_expr(cx, item))
            .collect::<Result<Vec<_>>>(),
        other => Ok(vec![value_from_expr(cx, &other)?]),
    }
}

fn tool_export_kind_symbol() -> Symbol {
    Symbol::new(TOOL_EXPORT_KIND)
}

pub(crate) fn tool_export_kind() -> ExportKind {
    ExportKind::new(tool_export_kind_symbol())
}
