use super::model::AgentManifest;
use super::ops::shared::wire_step_connection;
use super::tool_injection::{inject_manifest_tools, is_model_request};
use super::{component_name, ensure_task_id, first_codec, record_trace_entry, with_task_id};
use sim_kernel::{CapabilityName, Cx, Error, EvalReply, Result, Symbol, Value};
use sim_lib_server::{
    Connection, EvalSite, FrameKind, PipelineEvalSite, ServerAddress, ServerFrame,
    eval_reply_from_frame, eval_request_from_frame, server_frame_from_reply,
    server_frame_from_request,
};
use std::{any::Any, sync::Arc};

include!("helpers/runtime_sites.rs");
include!("helpers/manifest_runtime.rs");
