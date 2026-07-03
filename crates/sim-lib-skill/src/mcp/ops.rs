use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};

use super::tools::{McpCallParams, McpToolDescriptor, McpToolResult};

pub(crate) fn mcp_tools(cx: &mut Cx, args: Args) -> Result<Value> {
    if !args.into_vec().is_empty() {
        return Err(Error::Eval(
            "skill/mcp-tools expects no arguments".to_owned(),
        ));
    }
    let registry = crate::skill_registry(cx)?;
    let descriptors = registry
        .cards()?
        .iter()
        .filter(|card| card.roles.contains(&crate::SkillRole::Tool))
        .map(|card| Ok(McpToolDescriptor::from_skill_card(cx, card)?.to_expr()))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().expr(Expr::List(descriptors))
}

pub(crate) fn mcp_call(cx: &mut Cx, args: Args) -> Result<Value> {
    let params = one_arg(cx, args)?;
    let registry = crate::skill_registry(cx)?;
    let card = registry
        .card_by_id(&params.name)?
        .ok_or_else(|| Error::Eval(format!("unknown MCP skill tool {}", params.name)))?;
    let values = params
        .arguments
        .into_iter()
        .map(|expr| expr_arg_value(cx, expr))
        .collect::<Result<Vec<_>>>()?;
    let value = cx.call_function(&card.symbol, Args::new(values))?;
    McpToolResult::success(cx, value)?.to_value(cx)
}

fn one_arg(cx: &mut Cx, args: Args) -> Result<McpCallParams> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(
            "skill/mcp-call expects one MCP tools/call params value".to_owned(),
        ));
    }
    McpCallParams::from_value(cx, &values.remove(0))
}

fn expr_arg_value(cx: &mut Cx, expr: Expr) -> Result<Value> {
    match expr {
        Expr::Nil => cx.factory().nil(),
        Expr::Bool(value) => cx.factory().bool(value),
        Expr::Number(number) => cx.factory().number_literal(number.domain, number.canonical),
        Expr::String(value) => cx.factory().string(value),
        Expr::Symbol(value) => cx.factory().symbol(value),
        other => cx.factory().expr(other),
    }
}

/// Returns the symbol for the `skill/mcp-tools` operation.
pub fn skill_mcp_tools_symbol() -> Symbol {
    Symbol::qualified("skill", "mcp-tools")
}

/// Returns the symbol for the `skill/mcp-call` operation.
pub fn skill_mcp_call_symbol() -> Symbol {
    Symbol::qualified("skill", "mcp-call")
}
