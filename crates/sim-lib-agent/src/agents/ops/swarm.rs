use super::shared::{optional_value, values_option};
use crate::{
    SWARM_LAUNCH_CAPABILITY, installed_codecs, maybe_f64_option, maybe_u32_option,
    parse_component_options,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Result, Value};
use sim_lib_server::{Connection, ServerAddress};
use std::sync::{Arc, Mutex, atomic::Ordering};

use super::super::first_codec;
use super::super::model::{
    AgentFabric, NEXT_SWARM_ID, RuntimeValueSite, SwarmRegistry, fabric_from_value,
    swarm_explain_expr, swarm_status_value_for_table,
};

pub(crate) fn swarm_make_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "swarm/make")?;
    let name = crate::symbol_option(cx, &options, "name", sim_kernel::Symbol::new("swarm"))?;
    let budget = Some(super::super::model::Budget {
        max_turns: maybe_u32_option(cx, &options, "max-turns")?,
        max_cost: maybe_f64_option(cx, &options, "max-cost")?,
    })
    .filter(|budget| budget.max_turns.is_some() || budget.max_cost.is_some());
    let fabric = AgentFabric {
        id: NEXT_SWARM_ID.fetch_add(1, Ordering::Relaxed),
        name,
        members: values_option(cx, &options, "agents")?,
        topology: optional_value(&options, "topology"),
        planner: optional_value(&options, "planner"),
        blackboard: optional_value(&options, "blackboard"),
        budget,
        codecs: installed_codecs(cx),
        last_run: Mutex::new(None),
        runs: Arc::new(Mutex::new(SwarmRegistry {
            next_task_id: 1,
            ..SwarmRegistry::default()
        })),
    };
    cx.factory().opaque(Arc::new(fabric))
}

pub(crate) fn swarm_launch_value(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&CapabilityName::new(SWARM_LAUNCH_CAPABILITY))?;
    let [swarm_value, task, ..] = args.values() else {
        return Err(Error::Eval(
            "swarm/launch expects a swarm and a task".to_owned(),
        ));
    };
    let fabric = fabric_from_value(swarm_value)?;
    let expr = task.object().as_expr(cx)?;
    let result = fabric.realize_expr(cx, expr)?;
    cx.factory().expr(result)
}

pub(crate) fn swarm_explain_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [swarm_value, rest @ ..] = args.values() else {
        return Err(Error::Eval(
            "swarm/explain expects a swarm and optional task id".to_owned(),
        ));
    };
    let fabric = fabric_from_value(swarm_value)?;
    let task_id = rest.first().map(|value| {
        value.object().as_expr(cx).ok().and_then(|expr| match expr {
            sim_kernel::Expr::String(text) => Some(text),
            sim_kernel::Expr::Symbol(symbol) => Some(symbol.to_string()),
            _ => None,
        })
    });
    let explanation =
        swarm_explain_expr(fabric, task_id.as_ref().and_then(|value| value.as_deref()))?;
    cx.factory().expr(explanation)
}

pub(crate) fn swarm_status_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [swarm_value] = args.values() else {
        return Err(Error::Eval("swarm/status expects a swarm".to_owned()));
    };
    let fabric = fabric_from_value(swarm_value)?;
    swarm_status_value_for_table(cx, fabric)
}

pub(crate) fn swarm_as_fabric_value(_cx: &mut Cx, args: Args) -> Result<Value> {
    let [swarm_value] = args.values() else {
        return Err(Error::Eval("swarm/as-fabric expects a swarm".to_owned()));
    };
    Ok(swarm_value.clone())
}

pub(crate) fn swarm_as_site_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [swarm_value] = args.values() else {
        return Err(Error::Eval("swarm/as-site expects a swarm".to_owned()));
    };
    let fabric = fabric_from_value(swarm_value)?;
    let site = Arc::new(RuntimeValueSite {
        value: swarm_value.clone(),
        address: ServerAddress::Local,
        codecs: fabric.codecs.clone(),
        kind: "swarm",
    });
    let connection = Connection::with_session(
        ServerAddress::Local,
        first_codec(&fabric.codecs),
        fabric.codecs.clone(),
        site,
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?;
    cx.factory().opaque(Arc::new(connection))
}
