//! Local subprocess-backed model runners for SIM.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod broker;
mod claude;
mod codex;
mod effects;
mod json_stdio;
mod line_text;
mod opencode;
mod process;

use sim_codec_chat::model_error_expr;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{
    ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
};

pub use broker::BrokerSessionController;
pub use claude::{ClaudeCliAdapter, register_claude_cli};
pub use codex::{CodexCliAdapter, register_codex_cli};
pub use effects::host_process_capability;
pub use opencode::{OpenCodeCliAdapter, register_opencode_cli};
pub use process::{
    BrokerProcessSpec, ProcessExitReport, ProcessProgram, StderrSink, StdoutFraming,
    active_process_port, bind_process_port, frame_stdout, process_port_symbol, run_broker_process,
};

/// Wire protocol a [`ProcessRunner`] speaks with its subprocess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessProtocol {
    /// One JSON request/response exchange over stdin/stdout.
    JsonStdio,
    /// Plain text prompt in, line-delimited text out.
    LineText,
}

/// Local subprocess-backed [`ModelRunner`].
#[derive(Clone, Debug)]
pub struct ProcessRunner {
    runner: Symbol,
    model: String,
    process: BrokerProcessSpec,
    protocol: ProcessProtocol,
}

impl ProcessRunner {
    /// Builds a runner over one sealed broker process specification.
    pub fn new(
        runner: Symbol,
        model: impl Into<String>,
        process: BrokerProcessSpec,
        protocol: ProcessProtocol,
    ) -> Self {
        Self {
            runner,
            model: model.into(),
            process,
            protocol,
        }
    }

    fn infer_inner(&self, cx: &Cx, request: ModelRequest) -> Result<ModelResponse> {
        match self.protocol {
            ProcessProtocol::JsonStdio => json_stdio::infer(cx, self, request),
            ProcessProtocol::LineText => line_text::infer(cx, self, request),
        }
    }

    fn error_response(&self, message: impl Into<String>) -> ModelResponse {
        ModelResponse::try_from(model_error_expr(
            self.runner.clone(),
            self.model.clone(),
            message,
        ))
        .expect("model_error_expr should always produce a valid response transcript")
    }
}

impl ModelRunner for ProcessRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            self.runner.clone(),
            self.model.clone(),
            Symbol::new("process"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        match effects::resolve_process_effect(self, cx, request, |cx, runner, request| {
            runner.infer_inner(cx, request)
        }) {
            Ok(response) => Ok(response),
            Err(error) => Ok(self.error_response(render_error(error))),
        }
    }

    fn infer_stream(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        let streamed = match self.protocol {
            ProcessProtocol::JsonStdio => None,
            ProcessProtocol::LineText => Some(effects::resolve_process_effect(
                self,
                cx,
                request.clone(),
                {
                    let sink = &mut *sink;
                    |cx, runner, request| line_text::infer_stream(cx, runner, request, sink)
                },
            )),
        };
        match streamed.unwrap_or_else(|| {
            effects::resolve_process_effect(self, cx, request, |cx, runner, request| {
                runner.infer_inner(cx, request)
            })
        }) {
            Ok(response) => Ok(response),
            Err(error) => {
                let message = render_error(error);
                sink.emit(ModelEvent::error_text(
                    self.runner.clone(),
                    self.model.clone(),
                    Expr::String("process-error".to_owned()),
                    message.clone(),
                ))?;
                let response = self.error_response(message);
                sink.emit(ModelEvent::final_of(&response))?;
                Ok(response)
            }
        }
    }
}

fn render_error(error: Error) -> String {
    match error {
        Error::Eval(message) | Error::HostError(message) => message,
        other => other.to_string(),
    }
}

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
