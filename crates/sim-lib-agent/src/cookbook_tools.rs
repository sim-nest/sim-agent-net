use std::sync::Arc;

use sim_kernel::{
    CapabilityName, Cx, ExportKind, ExportRecord, ExportState, Result, RuntimeId, Symbol,
};
use sim_shape::{AnyShape, ListShape, shape_value};

use crate::{AGENT_LIB_ID, Tool, ToolSpec, install_tool};

pub(crate) fn install_cookbook_tools(cx: &mut Cx) -> Result<()> {
    sim_lib_cookbook::install_seeded_cookbook_lib(cx)?;
    let search_tool = cookbook_tool(cx, "search", "search seeded cookbook recipes", Vec::new())?;
    install_tool(cx, Arc::new(search_tool))?;
    let run_tool = cookbook_tool(
        cx,
        "run",
        "run a seeded cookbook recipe with expectation checks",
        vec![sim_kernel::read_eval_capability()],
    )?;
    install_tool(cx, Arc::new(run_tool))?;
    install_cookbook_card(cx)
}

fn cookbook_tool(
    cx: &mut Cx,
    name: &str,
    description: &str,
    capabilities: Vec<CapabilityName>,
) -> Result<Tool> {
    let function = cx.resolve_function(&Symbol::qualified("cookbook", name))?;
    Ok(Tool::local(
        cx,
        ToolSpec {
            symbol: Symbol::qualified("agent", format!("cookbook-{name}")),
            description: description.to_owned(),
            args_shape: one_arg_shape(name),
            result_shape: Some(any_shape(&format!("{name}-result"))),
            category: Symbol::new("cookbook"),
            capabilities,
            function,
        },
    ))
}

fn install_cookbook_card(cx: &mut Cx) -> Result<()> {
    let symbol = Symbol::qualified("agent", "cookbook");
    if cx.registry().value_by_symbol(&symbol).is_some() {
        return Ok(());
    }
    let search = cx
        .factory()
        .symbol(Symbol::qualified("agent", "cookbook-search"))?;
    let run = cx
        .factory()
        .symbol(Symbol::qualified("agent", "cookbook-run"))?;
    let value = cx.factory().table(vec![
        (
            Symbol::new("kind"),
            cx.factory()
                .symbol(Symbol::qualified("agent", "cookbook"))?,
        ),
        (Symbol::new("name"), cx.factory().symbol(symbol.clone())?),
        (
            Symbol::new("description"),
            cx.factory().string(
                "agent cookbook surface backed by cookbook:search and cookbook:run".to_owned(),
            )?,
        ),
        (
            Symbol::new("category"),
            cx.factory().symbol(Symbol::new("cookbook"))?,
        ),
        (Symbol::new("search"), search.clone()),
        (Symbol::new("run"), run.clone()),
        (Symbol::new("tools"), cx.factory().list(vec![search, run])?),
    ])?;
    cx.registry_mut()
        .register_value_for_lib(&Symbol::new(AGENT_LIB_ID), symbol.clone(), value)?;
    cx.registry_mut().append_export_record(
        &Symbol::new(AGENT_LIB_ID),
        ExportRecord {
            kind: ExportKind::new(Symbol::new("card")),
            symbol,
            state: ExportState::Resolved {
                id: RuntimeId::Value,
            },
        },
    )
}

fn one_arg_shape(name: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("agent/cookbook", format!("{name}-args")),
        Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
    )
}

fn any_shape(name: &str) -> sim_kernel::ShapeRef {
    shape_value(
        Symbol::qualified("agent/cookbook", name.to_owned()),
        Arc::new(AnyShape),
    )
}
