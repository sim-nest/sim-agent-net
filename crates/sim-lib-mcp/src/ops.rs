use std::sync::Arc;

use sim_kernel::{
    Args, Callable, Cx, Error, Export, Expr, Object, ObjectCompat, Result, Symbol, Value,
};
use sim_shape::{AnyShape, ListShape, Shape, shape_value};

use crate::McpRouter;
use crate::methods::{
    core, prompts as prompt_methods, resources as resource_methods, tools as tool_methods,
};

/// Selects which MCP method a [`McpFunction`] callable dispatches to.
#[derive(Clone, Copy)]
pub enum McpFunctionKind {
    /// Route a full encoded MCP envelope.
    Handle,
    /// Run the `initialize` handshake.
    Initialize,
    /// List available tools.
    Tools,
    /// Invoke a tool (`tools/call`).
    Call,
    /// List available resources.
    Resources,
    /// Read a resource (`resources/read`).
    Read,
    /// List available prompts.
    Prompts,
    /// Fetch a prompt (`prompts/get`).
    GetPrompt,
    /// Provide the sampling runner object.
    #[cfg(feature = "sampling")]
    SamplingRunner,
    /// Report server health.
    Health,
}

impl McpFunctionKind {
    /// Returns the runtime symbol naming this method.
    pub fn symbol(self) -> Symbol {
        match self {
            Self::Handle => handle_symbol(),
            Self::Initialize => initialize_symbol(),
            Self::Tools => tools_symbol(),
            Self::Call => call_symbol(),
            Self::Resources => resources_symbol(),
            Self::Read => read_symbol(),
            Self::Prompts => prompts_symbol(),
            Self::GetPrompt => get_prompt_symbol(),
            #[cfg(feature = "sampling")]
            Self::SamplingRunner => crate::sampling::mcp_sampling_runner_symbol(),
            Self::Health => health_symbol(),
        }
    }
}

/// Runtime callable object that dispatches one MCP method.
#[derive(Clone)]
pub struct McpFunction {
    kind: McpFunctionKind,
}

impl McpFunction {
    /// Creates a function callable for `kind`.
    pub fn new(kind: McpFunctionKind) -> Self {
        Self { kind }
    }

    /// Returns the runtime symbol naming this function.
    pub fn symbol(&self) -> Symbol {
        self.kind.symbol()
    }

    /// Creates a shared [`McpFunction`] for `kind`.
    pub fn value(kind: McpFunctionKind) -> Arc<Self> {
        Arc::new(Self::new(kind))
    }
}

impl Object for McpFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for McpFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for McpFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        match self.kind {
            McpFunctionKind::Handle => handle(cx, args),
            McpFunctionKind::Initialize => initialize(cx, args),
            McpFunctionKind::Tools => tools(cx, args),
            McpFunctionKind::Call => call(cx, args),
            McpFunctionKind::Resources => resources(cx, args),
            McpFunctionKind::Read => read(cx, args),
            McpFunctionKind::Prompts => prompts(cx, args),
            McpFunctionKind::GetPrompt => get_prompt(cx, args),
            #[cfg(feature = "sampling")]
            McpFunctionKind::SamplingRunner => sampling_runner(cx, args),
            McpFunctionKind::Health => health(cx, args),
        }
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<sim_kernel::ShapeRef>> {
        let shape: Arc<dyn Shape> = match self.kind {
            McpFunctionKind::Handle
            | McpFunctionKind::Call
            | McpFunctionKind::Read
            | McpFunctionKind::GetPrompt => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            McpFunctionKind::Initialize
            | McpFunctionKind::Tools
            | McpFunctionKind::Resources
            | McpFunctionKind::Prompts
            | McpFunctionKind::Health => Arc::new(ListShape::new(Vec::new())),
            #[cfg(feature = "sampling")]
            McpFunctionKind::SamplingRunner => Arc::new(ListShape::new(Vec::new())),
        };
        Ok(Some(shape_value(
            Symbol::qualified(self.symbol().to_string(), "args"),
            shape,
        )))
    }

    fn browse_result_shape(&self, _cx: &mut Cx) -> Result<Option<sim_kernel::ShapeRef>> {
        Ok(Some(shape_value(
            Symbol::qualified(self.symbol().to_string(), "result"),
            Arc::new(AnyShape),
        )))
    }
}

/// Returns the export records for every MCP method function.
pub fn mcp_exports() -> Vec<Export> {
    [
        handle_symbol(),
        initialize_symbol(),
        tools_symbol(),
        call_symbol(),
        resources_symbol(),
        read_symbol(),
        prompts_symbol(),
        get_prompt_symbol(),
        #[cfg(feature = "sampling")]
        crate::sampling::mcp_sampling_runner_symbol(),
        health_symbol(),
    ]
    .into_iter()
    .map(|symbol| Export::Function {
        symbol,
        function_id: None,
    })
    .collect()
}

fn handle(cx: &mut Cx, args: Args) -> Result<Value> {
    let expr = one_expr_arg(cx, args, "mcp/handle expects one decoded MCP envelope Expr")?;
    let mut router = McpRouter::fixture();
    match router.handle_expr(cx, expr)? {
        Some(reply) => cx.factory().expr(reply),
        None => cx.factory().nil(),
    }
}

fn initialize(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/initialize expects no arguments")?;
    let mut session = crate::McpSession::fixture();
    cx.factory()
        .expr(core::initialize(&mut session, Expr::Nil)?)
}

fn tools(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/tools expects no arguments")?;
    let session = crate::McpSession::fixture();
    let result = tool_methods::list(cx, &session)?;
    cx.factory().expr(result)
}

fn call(cx: &mut Cx, args: Args) -> Result<Value> {
    let params = one_expr_arg(cx, args, "mcp/call expects one tools/call params Expr")?;
    let session = crate::McpSession::fixture();
    let result = tool_methods::call(cx, &session, params)?;
    cx.factory().expr(result)
}

fn resources(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/resources expects no arguments")?;
    let session = crate::McpSession::fixture();
    let result = resource_methods::list(cx, &session)?;
    cx.factory().expr(result)
}

fn read(cx: &mut Cx, args: Args) -> Result<Value> {
    let params = one_expr_arg(cx, args, "mcp/read expects one resources/read params Expr")?;
    let session = crate::McpSession::fixture();
    let result = resource_methods::read(cx, &session, params)?;
    cx.factory().expr(result)
}

fn prompts(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/prompts expects no arguments")?;
    let session = crate::McpSession::fixture();
    let result = prompt_methods::list(cx, &session)?;
    cx.factory().expr(result)
}

fn get_prompt(cx: &mut Cx, args: Args) -> Result<Value> {
    let params = one_expr_arg(
        cx,
        args,
        "mcp/get-prompt expects one prompts/get params Expr",
    )?;
    let session = crate::McpSession::fixture();
    let result = prompt_methods::get(cx, &session, params)?;
    cx.factory().expr(result)
}

#[cfg(feature = "sampling")]
fn sampling_runner(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/sampling-runner expects no arguments")?;
    crate::sampling::sampling_runner_value(
        cx,
        Arc::new(crate::sampling::McpSamplingRunner::fixture()),
    )
}

fn health(cx: &mut Cx, args: Args) -> Result<Value> {
    no_args(args, "mcp/health expects no arguments")?;
    let session = crate::McpSession::fixture();
    cx.factory().expr(core::health(&session))
}

fn one_expr_arg(cx: &mut Cx, args: Args, message: &'static str) -> Result<Expr> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(message.to_owned()));
    }
    values.remove(0).object().as_expr(cx)
}

fn no_args(args: Args, message: &'static str) -> Result<()> {
    if args.values().is_empty() {
        Ok(())
    } else {
        Err(Error::Eval(message.to_owned()))
    }
}

/// Returns the `mcp/handle` function symbol.
pub fn handle_symbol() -> Symbol {
    Symbol::qualified("mcp", "handle")
}

/// Returns the `mcp/initialize` function symbol.
pub fn initialize_symbol() -> Symbol {
    Symbol::qualified("mcp", "initialize")
}

/// Returns the `mcp/tools` function symbol.
pub fn tools_symbol() -> Symbol {
    Symbol::qualified("mcp", "tools")
}

/// Returns the `mcp/call` function symbol.
pub fn call_symbol() -> Symbol {
    Symbol::qualified("mcp", "call")
}

/// Returns the `mcp/resources` function symbol.
pub fn resources_symbol() -> Symbol {
    Symbol::qualified("mcp", "resources")
}

/// Returns the `mcp/read` function symbol.
pub fn read_symbol() -> Symbol {
    Symbol::qualified("mcp", "read")
}

/// Returns the `mcp/prompts` function symbol.
pub fn prompts_symbol() -> Symbol {
    Symbol::qualified("mcp", "prompts")
}

/// Returns the `mcp/get-prompt` function symbol.
pub fn get_prompt_symbol() -> Symbol {
    Symbol::qualified("mcp", "get-prompt")
}

/// Returns the `mcp/health` function symbol.
pub fn health_symbol() -> Symbol {
    Symbol::qualified("mcp", "health")
}
