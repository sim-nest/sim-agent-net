use std::sync::Arc;

use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};

use crate::SkillCard;
use crate::registry::card_from_value;

pub(crate) fn as_tool(cx: &mut Cx, args: Args) -> Result<Value> {
    let card = card_arg(cx, args)?;
    sim_lib_agent::install_agent_lib(cx)?;
    let function = cx.resolve_function(&card.symbol)?;
    let tool = sim_lib_agent::Tool::local(
        cx,
        sim_lib_agent::ToolSpec {
            symbol: card.symbol.clone(),
            description: card.description.clone(),
            args_shape: card.input_shape.clone(),
            result_shape: Some(card.output_shape.clone()),
            category: Symbol::new("skill"),
            capabilities: card.capabilities.clone(),
            function,
        },
    );
    sim_lib_agent::install_tool(cx, Arc::new(tool))
}

/// Returns the symbol for the `skill/as-tool` operation.
pub fn skill_as_tool_symbol() -> Symbol {
    Symbol::qualified("skill", "as-tool")
}

fn card_arg(cx: &mut Cx, args: Args) -> Result<SkillCard> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(
            "skill/as-tool expects one skill id, symbol, or SkillCard value".to_owned(),
        ));
    }
    let value = values.remove(0);
    if value.object().downcast_ref::<SkillCard>().is_some() {
        return card_from_value(cx, &value);
    }
    match value.object().as_expr(cx)? {
        Expr::Map(_) => card_from_value(cx, &value),
        Expr::String(id) => crate::skill_registry(cx)?
            .card_by_id(&id)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {id}"))),
        Expr::Symbol(symbol) => crate::skill_registry(cx)?
            .card_by_symbol(&symbol)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {symbol}"))),
        _ => Err(Error::TypeMismatch {
            expected: "skill id, symbol, or SkillCard",
            found: "invalid target",
        }),
    }
}

#[cfg(test)]
mod tests;
