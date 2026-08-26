use super::agent_r18_support::{
    fixed_reply_connection, map_expr_field, register_bid_worker, register_connection,
    star_hub_connection, tagged_append_connection, verifier_connection,
};
use super::support::{eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs};
use crate::agents::topology_data::{
    build_debate_data_graph_connection, build_market_data_graph_connection,
    build_mesh_data_graph_connection, build_open_claw_data_graph_connection,
    build_ring_data_graph_connection, build_speculate_verify_data_graph_connection,
    build_star_data_graph_connection,
};
use sim_kernel::{Args, CapabilitySet, Consistency, Cx, EvalMode, EvalRequest, Expr, Symbol};
use sim_lib_agent_conduct_core::{AgentRunFrame, AgentUsageBudget, UsageQuantity};
use sim_lib_server::Connection;
use sim_lib_topology::topology_run_capability;

use crate::{execute_delegate_once, steps::DelegateRequest};

include!("agent_r23/topologies.rs");
include!("agent_r23/delegation.rs");
