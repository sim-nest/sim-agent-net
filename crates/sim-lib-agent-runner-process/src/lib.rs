//! Local subprocess-backed model runners for SIM.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod effects;
mod json_stdio;
mod line_text;
mod process;

use sim_codec_chat::model_error_expr;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{
    ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
};
use std::time::Duration;

pub use effects::host_process_capability;
pub use process::{ProcessCommandSpec, run_process_command, stream_process_command_lines};

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
    command: String,
    protocol: ProcessProtocol,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessRunner {
    /// Builds a runner that invokes `command` using `protocol`, bounded by
    /// `timeout` and `max_output_bytes`.
    pub fn new(
        runner: Symbol,
        model: impl Into<String>,
        command: impl Into<String>,
        protocol: ProcessProtocol,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            runner,
            model: model.into(),
            command: command.into(),
            protocol,
            timeout,
            max_output_bytes,
        }
    }

    fn infer_inner(&self, request: ModelRequest) -> Result<ModelResponse> {
        match self.protocol {
            ProcessProtocol::JsonStdio => json_stdio::infer(self, request),
            ProcessProtocol::LineText => line_text::infer(self, request),
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
        match effects::resolve_process_effect(self, cx, request, |runner, request| {
            runner.infer_inner(request)
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
                    |runner, request| line_text::infer_stream(runner, request, sink)
                },
            )),
        };
        match streamed.unwrap_or_else(|| {
            effects::resolve_process_effect(self, cx, request, |runner, request| {
                runner.infer_inner(request)
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
