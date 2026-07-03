use super::super::model::{AgentComponent, SandboxBackend};
use super::process::{capture_child_output, io_error_to_host, shell_child};
use crate::{SANDBOX_CAPABILITY, SANDBOX_SUBPROCESS_CAPABILITY, SANDBOX_WASM_CAPABILITY};
use sim_codec::encode_with_codec;
use sim_kernel::{CapabilityName, CapabilitySet, Cx, EncodeOptions, Error, Result, Symbol};
use sim_lib_server::{
    EvalSite, FrameKind, LocalEvalSite, ServerAddress, ServerFrame, connect_transport_site,
    eval_request_from_frame,
};
use std::{process::Command, process::Stdio, time::Duration};

pub(in crate::components) fn answer_sandbox(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &SandboxBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    match backend {
        SandboxBackend::Wasm { region } => {
            cx.require(&CapabilityName::new(SANDBOX_WASM_CAPABILITY))?;
            route_through_wasm_boundary(cx, frame, region)
        }
        SandboxBackend::Subprocess {
            command,
            max_time,
            max_output_bytes,
        } => {
            cx.require(&CapabilityName::new(SANDBOX_SUBPROCESS_CAPABILITY))?;
            answer_subprocess(cx, &frame, command.as_deref(), *max_time, *max_output_bytes)
        }
        SandboxBackend::CapabilityRestricted { allowed } => {
            cx.require(&CapabilityName::new(SANDBOX_CAPABILITY))?;
            answer_capability_restricted(cx, component, frame, allowed)
        }
    }
}

fn route_through_wasm_boundary(
    cx: &mut Cx,
    frame: ServerFrame,
    region: &str,
) -> Result<ServerFrame> {
    let address = ServerAddress::Wasm {
        region: region.to_owned(),
    };
    let (site, _) = connect_transport_site(cx, address, vec![frame.codec.clone()])?;
    site.answer(cx, frame)
}

fn answer_capability_restricted(
    cx: &mut Cx,
    component: &AgentComponent,
    frame: ServerFrame,
    allowed: &[CapabilityName],
) -> Result<ServerFrame> {
    let mut restricted = CapabilitySet::new();
    for capability in allowed {
        if cx.capabilities().contains(capability) {
            restricted.insert(capability.clone());
        }
    }
    let site = LocalEvalSite::new(ServerAddress::Local, component.codecs.clone());
    cx.with_capabilities(restricted, |cx| site.answer(cx, frame))
}

fn answer_subprocess(
    cx: &mut Cx,
    frame: &ServerFrame,
    command: Option<&str>,
    max_time: Duration,
    max_output_bytes: usize,
) -> Result<ServerFrame> {
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, frame)?;
    let source = encode_with_codec(
        cx,
        &Symbol::qualified("codec", "lisp"),
        &request.expr,
        EncodeOptions::default(),
    )?
    .into_text()?;
    let stdout = run_subprocess_eval(command, &source, max_time, max_output_bytes)?;
    let value = crate::util::expr_to_value(
        cx,
        &sim_kernel::Expr::Map(vec![
            (
                sim_kernel::Expr::Symbol(Symbol::new("status")),
                sim_kernel::Expr::String("ok".to_owned()),
            ),
            (
                sim_kernel::Expr::Symbol(Symbol::new("stdout")),
                sim_kernel::Expr::String(stdout),
            ),
        ]),
    )?;
    crate::reply::reply_frame(cx, frame, value, consistency)
}

fn run_subprocess_eval(
    command: Option<&str>,
    source: &str,
    max_time: Duration,
    max_output_bytes: usize,
) -> Result<String> {
    let child = spawn_subprocess(command, source)?;
    let bytes = capture_child_output(
        child,
        Vec::new(),
        "sandbox/subprocess",
        max_time,
        max_output_bytes,
    )?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn spawn_subprocess(command: Option<&str>, source: &str) -> Result<std::process::Child> {
    match command {
        Some(command) => {
            let mut cmd = shell_child(command);
            cmd.env("SIM_SANDBOX_EXPR", source);
            cmd.spawn().map_err(io_error_to_host)
        }
        None => {
            let mut cmd = Command::new("sim-server");
            cmd.arg("--eval").arg(source);
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::null())
                .stdin(Stdio::piped())
                .spawn()
                .map_err(io_error_to_host)
        }
    }
}
