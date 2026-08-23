mod driver;
mod helpers;
mod model;
mod ops;
mod tool_injection;
mod trace;

pub(crate) use driver::agent_line_driver_factory;
pub(crate) use helpers::{
    build_agent_runtime_site, collect_agent_components, component_kind_matches, component_name,
    conduct_id, first_codec, graph_fingerprint, required_roles_from_expr,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use model::LoopbackStream;
pub use model::{Agent, AgentFabric, AgentManifest, AgentRef, Budget, ComponentRef, TopologyRef};
pub(crate) use model::{resolve_agent_address, site_from_value};
#[cfg(test)]
pub(crate) use ops::topology_data;
pub(crate) use ops::{
    agent_attach_value, agent_audit_value, agent_call_value, agent_component_value,
    agent_components_value, agent_connect_value, agent_derive_value, agent_lisp_value,
    agent_make_value, agent_reflect_value, agent_replace_value, agent_restart_value,
    agent_server_value, agent_start_value, agent_stream_value, agent_trace_value, agent_wire_value,
    gateway_create_value, swarm_as_fabric_value, swarm_as_site_value, swarm_explain_value,
    swarm_launch_value, swarm_make_value, swarm_status_value, topology_debate_value,
    topology_market_value, topology_mesh_value, topology_open_claw_value, topology_ring_value,
    topology_speculate_verify_value, topology_star_value,
};
pub(crate) use trace::{
    audit_role_filter, ensure_task_id, parse_since_cutoff, record_trace_entry,
    remember_recorded_trace, with_task_id,
};
