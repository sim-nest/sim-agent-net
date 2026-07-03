use std::sync::Arc;

use sim_kernel::{
    Args, Callable, CapabilityName, Cx, Error, Export, Object, ObjectCompat, Result, Symbol, Value,
};
use sim_shape::{AnyShape, ListShape, Shape, shape_value};

use crate::registry::{card_from_value, transport_from_value};

/// Which `skill/*` operation a [`SkillFunction`] dispatches.
#[derive(Clone, Copy)]
pub enum SkillFunctionKind {
    /// `skill/install`: register a transport and bind its cards.
    Install,
    /// `skill/bind`: bind one or more cards to installed transports.
    Bind,
    /// `skill/list`: list the bound cards.
    List,
    /// `skill/card`: look up a single card by id or symbol.
    Card,
    /// `skill/call`: call a bound skill by id or symbol.
    Call,
    /// `skill/audit`: return the recorded cache/cassette audit log.
    #[cfg(any(feature = "cache", feature = "cassette"))]
    Audit,
    /// `skill/as-tool`: project a skill as an agent tool.
    #[cfg(feature = "agent")]
    AsTool,
    /// `skill/mcp-tools`: list skills as MCP tool descriptors.
    #[cfg(feature = "mcp")]
    McpTools,
    /// `skill/mcp-call`: invoke a skill through the MCP call surface.
    #[cfg(feature = "mcp")]
    McpCall,
    /// `skill/openai-tool`: project a skill as an OpenAI tool descriptor.
    #[cfg(feature = "openai")]
    OpenAiTool,
    /// `skill/openai-tools`: project skills as OpenAI tool descriptors.
    #[cfg(feature = "openai")]
    OpenAiTools,
    /// `skill/as-runner`: expose a model skill as an agent runner.
    #[cfg(feature = "runner")]
    AsRunner,
    /// `skill/serve-mcp`: serve bound skills over MCP.
    #[cfg(feature = "serve")]
    ServeMcp,
}

impl SkillFunctionKind {
    /// Returns the qualified symbol the operation is bound under.
    pub fn symbol(self) -> Symbol {
        match self {
            SkillFunctionKind::Install => skill_install_symbol(),
            SkillFunctionKind::Bind => skill_bind_symbol(),
            SkillFunctionKind::List => skill_list_symbol(),
            SkillFunctionKind::Card => skill_card_symbol(),
            SkillFunctionKind::Call => skill_call_symbol(),
            #[cfg(any(feature = "cache", feature = "cassette"))]
            SkillFunctionKind::Audit => skill_audit_symbol(),
            #[cfg(feature = "agent")]
            SkillFunctionKind::AsTool => crate::agent::skill_as_tool_symbol(),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpTools => crate::mcp::skill_mcp_tools_symbol(),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpCall => crate::mcp::skill_mcp_call_symbol(),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTool => crate::openai::skill_openai_tool_symbol(),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTools => crate::openai::skill_openai_tools_symbol(),
            #[cfg(feature = "runner")]
            SkillFunctionKind::AsRunner => crate::runner::skill_as_runner_symbol(),
            #[cfg(feature = "serve")]
            SkillFunctionKind::ServeMcp => crate::serve::skill_serve_mcp_symbol(),
        }
    }
}

/// Callable runtime object implementing one `skill/*` operation.
#[derive(Clone)]
pub struct SkillFunction {
    kind: SkillFunctionKind,
}

impl SkillFunction {
    /// Creates a function value for the given operation `kind`.
    pub fn new(kind: SkillFunctionKind) -> Self {
        Self { kind }
    }

    /// Returns the symbol this function is bound under.
    pub fn symbol(&self) -> Symbol {
        self.kind.symbol()
    }

    /// Creates a shared function object for the given operation `kind`.
    pub fn value(kind: SkillFunctionKind) -> Arc<Self> {
        Arc::new(Self::new(kind))
    }
}

impl Object for SkillFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for SkillFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for SkillFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        match self.kind {
            SkillFunctionKind::Install => install(cx, args),
            SkillFunctionKind::Bind => bind(cx, args),
            SkillFunctionKind::List => list(cx),
            SkillFunctionKind::Card => card(cx, args),
            SkillFunctionKind::Call => call(cx, args),
            #[cfg(any(feature = "cache", feature = "cassette"))]
            SkillFunctionKind::Audit => audit(cx),
            #[cfg(feature = "agent")]
            SkillFunctionKind::AsTool => crate::agent::as_tool(cx, args),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpTools => crate::mcp::ops::mcp_tools(cx, args),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpCall => crate::mcp::ops::mcp_call(cx, args),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTool => crate::openai::openai_tool(cx, args),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTools => crate::openai::openai_tools(cx, args),
            #[cfg(feature = "runner")]
            SkillFunctionKind::AsRunner => crate::runner::as_runner(cx, args),
            #[cfg(feature = "serve")]
            SkillFunctionKind::ServeMcp => crate::serve::serve_mcp(cx, args),
        }
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<sim_kernel::ShapeRef>> {
        let shape: Arc<dyn Shape> = match self.kind {
            SkillFunctionKind::List => Arc::new(ListShape::new(Vec::new())),
            #[cfg(any(feature = "cache", feature = "cassette"))]
            SkillFunctionKind::Audit => Arc::new(ListShape::new(Vec::new())),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpTools => Arc::new(ListShape::new(Vec::new())),
            SkillFunctionKind::Card => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            SkillFunctionKind::Install | SkillFunctionKind::Bind | SkillFunctionKind::Call => {
                Arc::new(AnyShape)
            }
            #[cfg(feature = "agent")]
            SkillFunctionKind::AsTool => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            #[cfg(feature = "mcp")]
            SkillFunctionKind::McpCall => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTool => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            #[cfg(feature = "openai")]
            SkillFunctionKind::OpenAiTools => Arc::new(AnyShape),
            #[cfg(feature = "runner")]
            SkillFunctionKind::AsRunner => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            #[cfg(feature = "serve")]
            SkillFunctionKind::ServeMcp => Arc::new(AnyShape),
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

/// Returns the library exports: every `skill/*` function plus the registry
/// value, with the feature-gated operations included when their feature is on.
pub fn skill_exports() -> Vec<Export> {
    let symbols = vec![
        skill_install_symbol(),
        skill_bind_symbol(),
        skill_list_symbol(),
        skill_card_symbol(),
        skill_call_symbol(),
    ];
    #[cfg(any(feature = "cache", feature = "cassette"))]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(skill_audit_symbol());
        symbols
    };
    #[cfg(feature = "agent")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(crate::agent::skill_as_tool_symbol());
        symbols
    };
    #[cfg(feature = "mcp")]
    let symbols = {
        let mut symbols = symbols;
        symbols.extend([
            crate::mcp::skill_mcp_tools_symbol(),
            crate::mcp::skill_mcp_call_symbol(),
        ]);
        symbols
    };
    #[cfg(feature = "openai")]
    let symbols = {
        let mut symbols = symbols;
        symbols.extend([
            crate::openai::skill_openai_tool_symbol(),
            crate::openai::skill_openai_tools_symbol(),
        ]);
        symbols
    };
    #[cfg(feature = "runner")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(crate::runner::skill_as_runner_symbol());
        symbols
    };
    #[cfg(feature = "serve")]
    let symbols = {
        let mut symbols = symbols;
        symbols.push(crate::serve::skill_serve_mcp_symbol());
        symbols
    };
    symbols
        .into_iter()
        .map(|symbol| Export::Function {
            symbol,
            function_id: None,
        })
        .chain(std::iter::once(Export::Value {
            symbol: crate::skill_registry_symbol(),
        }))
        .collect()
}

fn install(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&skill_install_capability())?;
    let values = args.into_vec();
    let Some((transport_value, card_values)) = values.split_first() else {
        return Err(Error::Eval(
            "skill/install expects a transport and one or more SkillCard values".to_owned(),
        ));
    };
    let registry = crate::skill_registry(cx)?;
    registry.install_transport(transport_from_value(transport_value)?)?;
    bind_cards(cx, &registry, card_values)
}

fn bind(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&skill_bind_capability())?;
    let values = args.into_vec();
    let registry = crate::skill_registry(cx)?;
    bind_cards(cx, &registry, &values)
}

fn bind_cards(
    cx: &mut Cx,
    registry: &crate::SkillRegistry,
    card_values: &[Value],
) -> Result<Value> {
    if card_values.is_empty() {
        return Err(Error::Eval(
            "skill/bind expects one or more SkillCard values".to_owned(),
        ));
    }
    let cards = card_values
        .iter()
        .map(|value| card_from_value(cx, value))
        .collect::<Result<Vec<_>>>()?;
    for card in &cards {
        registry.bind_card(cx, card.clone())?;
    }
    let values = cards
        .iter()
        .map(|card| card.value(cx))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(values)
}

fn list(cx: &mut Cx) -> Result<Value> {
    let registry = crate::skill_registry(cx)?;
    let cards = registry.cards()?;
    let values = cards
        .iter()
        .map(|card| card.value(cx))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(values)
}

fn card(cx: &mut Cx, args: Args) -> Result<Value> {
    let target = one_arg(args, "skill/card expects a skill id or symbol")?;
    let registry = crate::skill_registry(cx)?;
    let found = match target_from_value(cx, target)? {
        SkillTarget::Id(id) => registry.card_by_id(&id)?,
        SkillTarget::Symbol(symbol) => registry.card_by_symbol(&symbol)?,
    };
    match found {
        Some(card) => card.value(cx),
        None => cx.factory().nil(),
    }
}

fn call(cx: &mut Cx, args: Args) -> Result<Value> {
    let values = args.into_vec();
    let Some((target, rest)) = values.split_first() else {
        return Err(Error::Eval(
            "skill/call expects a skill id or symbol followed by arguments".to_owned(),
        ));
    };
    let registry = crate::skill_registry(cx)?;
    let card = match target_from_value(cx, target.clone())? {
        SkillTarget::Id(id) => registry.card_by_id(&id)?,
        SkillTarget::Symbol(symbol) => registry.card_by_symbol(&symbol)?,
    }
    .ok_or_else(|| Error::Eval("unknown skill".to_owned()))?;
    cx.call_function(&card.symbol, Args::new(rest.to_vec()))
}

#[cfg(any(feature = "cache", feature = "cassette"))]
fn audit(cx: &mut Cx) -> Result<Value> {
    cx.require(&skill_audit_capability())?;
    crate::skill_registry(cx)?.audit_values(cx)
}

fn one_arg(args: Args, message: &'static str) -> Result<Value> {
    let mut values = args.into_vec();
    if values.len() == 1 {
        Ok(values.remove(0))
    } else {
        Err(Error::Eval(message.to_owned()))
    }
}

enum SkillTarget {
    Id(String),
    Symbol(Symbol),
}

fn target_from_value(cx: &mut Cx, value: Value) -> Result<SkillTarget> {
    match value.object().as_expr(cx)? {
        sim_kernel::Expr::String(id) => Ok(SkillTarget::Id(id)),
        sim_kernel::Expr::Symbol(symbol) => Ok(SkillTarget::Symbol(symbol)),
        _ => Err(Error::TypeMismatch {
            expected: "skill id or symbol",
            found: "invalid target",
        }),
    }
}

/// Returns the symbol for the `skill/install` operation.
pub fn skill_install_symbol() -> Symbol {
    Symbol::qualified("skill", "install")
}

/// Returns the symbol for the `skill/bind` operation.
pub fn skill_bind_symbol() -> Symbol {
    Symbol::qualified("skill", "bind")
}

/// Returns the symbol for the `skill/list` operation.
pub fn skill_list_symbol() -> Symbol {
    Symbol::qualified("skill", "list")
}

/// Returns the symbol for the `skill/card` operation.
pub fn skill_card_symbol() -> Symbol {
    Symbol::qualified("skill", "card")
}

/// Returns the symbol for the `skill/call` operation.
pub fn skill_call_symbol() -> Symbol {
    Symbol::qualified("skill", "call")
}

/// Returns the symbol for the `skill/audit` operation.
#[cfg(any(feature = "cache", feature = "cassette"))]
pub fn skill_audit_symbol() -> Symbol {
    Symbol::qualified("skill", "audit")
}

/// Returns the capability required to call `skill/install`.
pub fn skill_install_capability() -> CapabilityName {
    CapabilityName::new("skill.install")
}

/// Returns the capability required to call `skill/bind`.
pub fn skill_bind_capability() -> CapabilityName {
    CapabilityName::new("skill.bind")
}

/// Returns the broad capability required to call any skill.
pub fn skill_call_capability() -> CapabilityName {
    CapabilityName::new("skill.call")
}

/// Returns the capability required to serve skills over a transport.
pub fn skill_serve_capability() -> CapabilityName {
    CapabilityName::new("skill.serve")
}

/// Returns the capability required to read the skill audit log.
#[cfg(any(feature = "cache", feature = "cassette"))]
pub fn skill_audit_capability() -> CapabilityName {
    CapabilityName::new("skill.audit")
}

/// Returns the per-skill call capability for the skill with the given `id`.
pub fn skill_specific_call_capability(id: &str) -> CapabilityName {
    CapabilityName::new(format!("skill.{id}.call"))
}
