//! Reusable, single-attempt agent model and tool step boundaries.

use std::{collections::BTreeMap, sync::Arc};

use sim_codec_bridge::BridgePacket;
use sim_kernel::{CapabilityName, Cx, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_conduct::run_frame_shape;
use sim_lib_agent_conduct_core::{
    AgentEvent, AgentRunFrame, AgentStepCard, AgentUsageBudget, UsageQuantity,
};
use sim_lib_agent_runner_core::{ModelResponse, ModelRunner};
use sim_lib_bridge::{AskAttempt, run_ask_once};
use sim_lib_provider::{ProviderRegistry, ProviderSeatId};
use sim_value::build::entry as field;

use crate::util::value_from_expr;
use crate::{Component, ComponentKind, PlanningOutput, PlanningTask, planning};

include!("steps/core.rs");
include!("steps/model_turn.rs");
