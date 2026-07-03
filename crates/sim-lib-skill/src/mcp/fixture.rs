use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};

use crate::{FixtureBehavior, SkillCard, SkillEventSink, SkillTransport};

use super::tools::McpToolDescriptor;

#[derive(Clone, Debug)]
struct FixtureMcpTool {
    descriptor: McpToolDescriptor,
    behavior: FixtureBehavior,
}

/// Deterministic in-memory MCP skill transport for tests.
///
/// Holds a set of MCP tool descriptors, each paired with a
/// [`FixtureBehavior`], and serves the MCP `initialize` and `tools/list`
/// surfaces along with the standard skill transport protocol.
pub struct FixtureMcpTransport {
    id: String,
    tools: Mutex<BTreeMap<String, FixtureMcpTool>>,
    calls: AtomicUsize,
}

impl FixtureMcpTransport {
    /// Creates an empty fixture MCP transport with the given `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tools: Mutex::new(BTreeMap::new()),
            calls: AtomicUsize::new(0),
        }
    }

    /// Registers a tool `descriptor` with the `behavior` to run when called.
    pub fn insert_tool(
        &self,
        descriptor: McpToolDescriptor,
        behavior: FixtureBehavior,
    ) -> Result<()> {
        self.tools
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP transport"))?
            .insert(
                descriptor.name.clone(),
                FixtureMcpTool {
                    descriptor,
                    behavior,
                },
            );
        Ok(())
    }

    /// Returns the number of calls dispatched through this transport.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Returns the MCP `initialize` response expression for this transport.
    pub fn initialize_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::qualified("mcp", "initialize"))),
            field("protocolVersion", Expr::String("2025-06-18".to_owned())),
            field("server", Expr::String(self.id.clone())),
        ])
    }

    /// Returns the MCP `tools/list` response expression listing every tool.
    pub fn tools_list_expr(&self) -> Result<Expr> {
        let tools = self
            .tools
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP transport"))?
            .values()
            .map(|tool| tool.descriptor.to_expr())
            .collect::<Vec<_>>();
        Ok(Expr::List(tools))
    }
}

impl SkillTransport for FixtureMcpTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "mcp"
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        self.tools
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP transport"))?
            .values()
            .map(|tool| tool.descriptor.to_skill_card(self.id.clone()))
            .collect()
    }

    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        let behavior = self
            .tools
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP transport"))?
            .get(&card.operation)
            .map(|tool| tool.behavior.clone())
            .ok_or_else(|| {
                Error::Eval(format!(
                    "fixture MCP transport {} has no tool {}",
                    self.id, card.operation
                ))
            })?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        call_behavior(cx, behavior, args)
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/mcp-health"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
            (
                Symbol::new("calls"),
                cx.factory().string(self.call_count().to_string())?,
            ),
        ])
    }
}

fn call_behavior(cx: &mut Cx, behavior: FixtureBehavior, args: Value) -> Result<Value> {
    match behavior {
        FixtureBehavior::EchoArgs => Ok(args),
        FixtureBehavior::SumNumbers => sum_number_args(cx, args),
        FixtureBehavior::ConstantString(text) => cx.factory().string(text),
    }
}

fn sum_number_args(cx: &mut Cx, args: Value) -> Result<Value> {
    let Expr::List(items) = args.object().as_expr(cx)? else {
        return Err(Error::TypeMismatch {
            expected: "argument list",
            found: "non-list",
        });
    };
    let mut sum = 0.0;
    for item in items {
        let Expr::Number(number) = item else {
            return Err(Error::TypeMismatch {
                expected: "number",
                found: "non-number",
            });
        };
        sum += number
            .canonical
            .parse::<f64>()
            .map_err(|err| Error::Eval(format!("invalid number literal: {err}")))?;
    }
    let canonical = if sum.fract() == 0.0 {
        format!("{}", sum as i64)
    } else {
        sum.to_string()
    };
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), canonical)
}

use sim_value::build::entry as field;
