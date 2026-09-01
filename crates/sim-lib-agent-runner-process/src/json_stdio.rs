use crate::{ProcessRunner, run_broker_process};
use serde_json::Value as JsonValue;
use sim_codec::DecodeBudget;
use sim_codec_json::{expr_to_json, json_to_expr};
use sim_kernel::{CodecId, Cx, Error, Expr, Result};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};
use sim_lib_exec::ProcessCancellation;

pub(crate) fn infer(
    cx: &Cx,
    runner: &ProcessRunner,
    request: ModelRequest,
) -> Result<ModelResponse> {
    let request_expr: Expr = request.into();
    let request_json = expr_to_json(&request_expr);
    let stdin =
        serde_json::to_vec(&request_json).map_err(|err| Error::HostError(err.to_string()))?;
    let stdout = run_broker_process(cx, &runner.process, stdin, &ProcessCancellation::default())?;
    let response_json: JsonValue = serde_json::from_slice(&stdout)
        .map_err(|err| Error::Eval(format!("runner/process returned invalid json: {err}")))?;
    let mut budget = DecodeBudget::new(Default::default());
    let response_expr = json_to_expr(CodecId(0), &response_json, &mut budget, 0)?;
    ModelResponse::try_from(response_expr)
}
