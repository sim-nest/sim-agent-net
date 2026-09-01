use super::shared::values_option;
use super::topology_data::{
    build_debate_data_graph_connection, build_market_data_graph_connection,
    build_mesh_data_graph_connection, build_open_claw_data_graph_connection,
    build_ring_data_graph_connection, build_speculate_verify_data_graph_connection,
    build_star_data_graph_connection,
};
use crate::{maybe_u32_option, parse_component_options, symbol_from_value, symbol_option};
use sim_kernel::{Args, Cx, Error, Result, Symbol, Value};
use sim_lib_server::{Connection, EvalSite, LocalEvalSite, ServerAddress};
use std::sync::Arc;

use super::super::first_codec;

pub(crate) fn topology_ring_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/ring")?;
    let agents = values_option(cx, &options, "agents")?;
    let role_cycle = role_cycle_option(cx, &options)?;
    let max_turns = maybe_u32_option(cx, &options, "max-turns")?
        .unwrap_or_else(|| u32::try_from(agents.len().max(1)).unwrap_or(u32::MAX));
    let connection = build_ring_data_graph_connection(cx, agents, role_cycle, max_turns)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_star_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/star")?;
    let hub = options
        .get("hub")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/star requires :hub".to_owned()))?;
    let spokes = values_option(cx, &options, "spokes")?;
    let hub_role = symbol_option(cx, &options, "hub-role", Symbol::new("planner"))?;
    let spoke_role = symbol_option(cx, &options, "spoke-role", Symbol::new("worker"))?;
    let connection = build_star_data_graph_connection(cx, hub, spokes, hub_role, spoke_role)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_mesh_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/mesh")?;
    let agents = values_option(cx, &options, "agents")?;
    let judge = options
        .get("judge")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/mesh requires :judge".to_owned()))?;
    let max_rounds = maybe_u32_option(cx, &options, "max-rounds")?.unwrap_or(2);
    let connection = build_mesh_data_graph_connection(cx, agents, judge, max_rounds)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_market_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/market")?;
    let workers = values_option(cx, &options, "workers")?;
    let router = options
        .get("router")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/market requires :router".to_owned()))?;
    let connection = build_market_data_graph_connection(cx, workers, router)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_debate_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/debate")?;
    let pro = options
        .get("pro")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/debate requires :pro".to_owned()))?;
    let con = options
        .get("con")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/debate requires :con".to_owned()))?;
    let judge = options
        .get("judge")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/debate requires :judge".to_owned()))?;
    let rounds = maybe_u32_option(cx, &options, "rounds")?.unwrap_or(1);
    let connection = build_debate_data_graph_connection(cx, pro, con, judge, rounds)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_speculate_verify_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/speculate-verify")?;
    let speculator = options
        .get("speculator")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/speculate-verify requires :speculator".to_owned()))?;
    let verifier = options
        .get("verifier")
        .cloned()
        .ok_or_else(|| Error::Eval("topology/speculate-verify requires :verifier".to_owned()))?;
    let on_mismatch = mismatch_policy(cx, &options)?;
    let connection =
        build_speculate_verify_data_graph_connection(cx, speculator, verifier, on_mismatch)?;
    cx.factory().opaque(connection)
}

pub(crate) fn topology_open_claw_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "topology/open-claw")?;
    let steps = topology_steps(cx, &options)?;
    if steps.is_empty() {
        return Err(Error::Eval(
            "topology/open-claw requires at least one step".to_owned(),
        ));
    }
    let connection = build_open_claw_data_graph_connection(cx, steps)?;
    cx.factory().opaque(connection)
}

pub(crate) fn gateway_create_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "gateway/create")?;
    let address = match options.get("address") {
        Some(value) => ServerAddress::from_expr(&value.object().as_expr(cx)?)?,
        None => ServerAddress::Local,
    };
    let codecs = crate::installed_codecs(cx);
    let site: Arc<dyn EvalSite> = Arc::new(LocalEvalSite::new(address.clone(), codecs.clone()));
    let connection = Connection::with_session(
        address,
        first_codec(&codecs),
        codecs,
        site,
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?;
    cx.factory().opaque(Arc::new(connection))
}

fn topology_steps(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Vec<Value>> {
    let mut steps = values_option(cx, options, "steps")?;
    steps.extend(values_option(cx, options, "agents")?);
    if let Some(hub) = options.get("hub") {
        steps.push(hub.clone());
    }
    steps.extend(values_option(cx, options, "spokes")?);
    if let Some(pro) = options.get("pro") {
        steps.push(pro.clone());
    }
    if let Some(con) = options.get("con") {
        steps.push(con.clone());
    }
    if let Some(judge) = options.get("judge") {
        steps.push(judge.clone());
    }
    if let Some(speculator) = options.get("speculator") {
        steps.push(speculator.clone());
    }
    if let Some(verifier) = options.get("verifier") {
        steps.push(verifier.clone());
    }
    if let Some(router) = options.get("router") {
        steps.push(router.clone());
    }
    Ok(steps)
}

fn role_cycle_option(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Vec<Symbol>> {
    let Some(value) = options.get("role-cycle") else {
        return Ok(vec![
            Symbol::new("planner"),
            Symbol::new("worker"),
            Symbol::new("critic"),
        ]);
    };
    match value.object().as_expr(cx)? {
        sim_kernel::Expr::List(items) | sim_kernel::Expr::Vector(items) => items
            .into_iter()
            .map(|expr| match expr {
                sim_kernel::Expr::Symbol(symbol) => Ok(symbol),
                other => Err(Error::Eval(format!(
                    "topology/ring :role-cycle expects symbols, found {other:?}"
                ))),
            })
            .collect(),
        sim_kernel::Expr::Symbol(symbol) => Ok(vec![symbol]),
        other => Err(Error::Eval(format!(
            "topology/ring :role-cycle expects a symbol list, found {other:?}"
        ))),
    }
}

fn mismatch_policy(
    cx: &mut Cx,
    options: &std::collections::HashMap<String, Value>,
) -> Result<Symbol> {
    match options.get("on-mismatch") {
        Some(value) => symbol_from_value(
            cx,
            value.clone(),
            "topology/speculate-verify :on-mismatch expects a symbol",
        ),
        None => Ok(Symbol::new("retry")),
    }
}
