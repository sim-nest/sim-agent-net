use sim_kernel::{ClassRef, Cx, Error, Object, ObjectCompat, Result, Symbol};
use sim_lib_exec::{
    ArgAtom, PrivateArtifactRef, ProcessAttempt, ProcessBudget, ProcessCancellation, ProcessPort,
    ProcessRequest, ProgramRef, ProjectRootRef, SealedBindings,
};
use std::{any::Any, sync::Arc, time::Duration};

/// Symbol of the lexical binding that carries the active process capsule.
pub fn process_port_symbol() -> Symbol {
    Symbol::qualified("agent", "process-port")
}

/// A broker seat's complete, portable process configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerProcessSpec {
    request: ProcessRequest,
    label: String,
}

impl BrokerProcessSpec {
    /// Creates a seat specification from opaque resources and literal arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program: ProgramRef,
        argv: Vec<ArgAtom>,
        root: ProjectRootRef,
        environment: SealedBindings,
        private_artifacts: Vec<PrivateArtifactRef>,
        label: impl Into<String>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self> {
        let timeout_ms = u64::try_from(timeout.as_millis())
            .map_err(|_| Error::Eval("process timeout exceeds the portable budget".into()))?;
        if timeout_ms == 0 {
            return Err(Error::Eval("process timeout must be non-zero".into()));
        }
        if max_output_bytes == 0 {
            return Err(Error::Eval("process output bound must be non-zero".into()));
        }
        Ok(Self {
            request: ProcessRequest {
                program,
                argv,
                root,
                environment,
                private_artifacts,
                budget: ProcessBudget {
                    timeout_ms,
                    max_output_bytes,
                    stdin: None,
                },
            },
            label: label.into(),
        })
    }

    /// Returns the exact request template owned by this seat.
    pub fn request(&self) -> &ProcessRequest {
        &self.request
    }

    fn request_with_stdin(&self, stdin: Vec<u8>) -> ProcessRequest {
        let mut request = self.request.clone();
        request.budget.stdin = Some(stdin);
        request
    }
}

/// Output framing admitted by the broker adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdoutFraming {
    /// Preserve the complete bounded stdout value.
    Whole,
    /// Split stdout into text lines.
    Lines,
    /// Split stdout into validated JSON lines.
    JsonLines,
}

/// Binds a process port in the active lexical environment.
pub fn bind_process_port(cx: &mut Cx, port: Arc<dyn ProcessPort>) -> Result<()> {
    let value = cx.factory().opaque(Arc::new(ProcessPortBinding { port }))?;
    cx.env_mut().define(process_port_symbol(), value);
    Ok(())
}

/// Returns the process port in the active lexical environment.
pub fn active_process_port(cx: &Cx) -> Result<Arc<dyn ProcessPort>> {
    cx.env()
        .get(&process_port_symbol())
        .and_then(|value| {
            value
                .object()
                .downcast_ref::<ProcessPortBinding>()
                .map(|binding| Arc::clone(&binding.port))
        })
        .ok_or_else(|| Error::HostError("provider refused: no active process port is bound".into()))
}

/// Runs a broker request exclusively through the active process port.
pub fn run_broker_process(
    cx: &Cx,
    spec: &BrokerProcessSpec,
    stdin: Vec<u8>,
    cancellation: &ProcessCancellation,
) -> Result<Vec<u8>> {
    let port = active_process_port(cx)?;
    let request = spec.request_with_stdin(stdin);
    let result = match port.run(&request, cancellation) {
        ProcessAttempt::Completed { receipt } => receipt.result,
        ProcessAttempt::NotDispatched { refusal } => {
            return Err(Error::HostError(format!(
                "{} provider refused before dispatch: {refusal:?}",
                spec.label
            )));
        }
        ProcessAttempt::StoppedAfterTimeout { .. } => {
            return Err(Error::Eval(format!("{} timed out", spec.label)));
        }
        ProcessAttempt::StoppedAfterCancel { .. } => {
            return Err(Error::Eval(format!("{} was cancelled", spec.label)));
        }
        ProcessAttempt::UnknownAfterDispatch { evidence } => {
            return Err(Error::HostError(format!(
                "{} process outcome is unknown after dispatch at {}: {}",
                spec.label, evidence.stage, evidence.detail
            )));
        }
    };
    if result.truncated {
        return Err(Error::Eval(format!(
            "{} exceeded max output bytes {}",
            spec.label, request.budget.max_output_bytes
        )));
    }
    if result.exit_code != 0 {
        return Err(Error::Eval(format!(
            "{} exited with status {}",
            spec.label, result.exit_code
        )));
    }
    Ok(result.stdout.into_bytes())
}

/// Frames already bounded stdout. Unsupported framing names fail closed.
pub fn frame_stdout(stdout: Vec<u8>, framing: &str) -> Result<Vec<Vec<u8>>> {
    match framing {
        "whole" => Ok(vec![stdout]),
        "lines" => Ok(stdout
            .split_inclusive(|byte| *byte == b'\n')
            .map(<[u8]>::to_vec)
            .collect()),
        "json-lines" => json_lines(&stdout),
        other => Err(Error::Eval(format!("unknown stdout framing {other}"))),
    }
}

fn json_lines(stdout: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut lines = Vec::new();
    for raw in stdout.split_inclusive(|byte| *byte == b'\n') {
        let payload = raw.strip_suffix(b"\n").unwrap_or(raw);
        let payload = payload.strip_suffix(b"\r").unwrap_or(payload);
        if payload.is_empty() {
            continue;
        }
        serde_json::from_slice::<serde_json::Value>(payload)
            .map_err(|error| Error::Eval(format!("invalid JSON line framing: {error}")))?;
        lines.push(raw.to_vec());
    }
    Ok(lines)
}

struct ProcessPortBinding {
    port: Arc<dyn ProcessPort>,
}
impl Object for ProcessPortBinding {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<process-port>".into())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl ObjectCompat for ProcessPortBinding {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_lib_exec::{ProcResult, ProcessReceipt, ProcessRefusal, StopReceipt};
    use std::sync::Mutex;

    struct ModelPort {
        requests: Mutex<Vec<ProcessRequest>>,
        attempts: Mutex<Vec<ProcessAttempt>>,
    }

    impl ModelPort {
        fn new(attempts: Vec<ProcessAttempt>) -> Self {
            Self {
                requests: Mutex::default(),
                attempts: Mutex::new(attempts.into_iter().rev().collect()),
            }
        }
    }

    impl ProcessPort for ModelPort {
        fn run(
            &self,
            request: &ProcessRequest,
            cancellation: &ProcessCancellation,
        ) -> ProcessAttempt {
            self.requests.lock().unwrap().push(request.clone());
            if cancellation.is_cancelled() {
                return ProcessAttempt::StoppedAfterCancel { receipt: stop() };
            }
            self.attempts.lock().unwrap().pop().unwrap()
        }
    }

    fn completed(stdout: &str, exit_code: i32, truncated: bool) -> ProcessAttempt {
        ProcessAttempt::Completed {
            receipt: ProcessReceipt {
                provider: "model".into(),
                elapsed_mono_ns: 1,
                result: ProcResult {
                    stdout: stdout.into(),
                    stderr: String::new(),
                    exit_code,
                    truncated,
                },
            },
        }
    }

    fn stop() -> StopReceipt {
        StopReceipt {
            provider: "model".into(),
            elapsed_mono_ns: 1,
            cleanup: "reaped".into(),
        }
    }

    fn spec(root: &str, artifact: &str) -> BrokerProcessSpec {
        BrokerProcessSpec::new(
            ProgramRef::new("provider-cli").unwrap(),
            vec![
                ArgAtom::new("spaces stay whole").unwrap(),
                ArgAtom::new("a'\"b").unwrap(),
            ],
            ProjectRootRef::new(root).unwrap(),
            SealedBindings::try_from_entries([(
                "CONFIG_HOME".into(),
                sim_lib_exec::BindingValue::PrivateArtifact(
                    PrivateArtifactRef::new(artifact).unwrap(),
                ),
            )])
            .unwrap(),
            vec![PrivateArtifactRef::new(artifact).unwrap()],
            "provider-seat",
            Duration::from_millis(25),
            64,
        )
        .unwrap()
    }

    fn cx_with(port: Arc<ModelPort>) -> Cx {
        let mut cx = test_cx();
        bind_process_port(&mut cx, port).unwrap();
        cx
    }

    fn test_cx() -> Cx {
        Cx::new(
            Arc::new(sim_kernel::eval::NoopEvalPolicy),
            Arc::new(sim_kernel::DefaultFactory),
        )
    }

    #[test]
    fn exact_request_has_literal_argv_sealed_mount_and_no_ambient_environment() {
        let port = Arc::new(ModelPort::new(vec![completed("ok", 0, false)]));
        let cx = cx_with(Arc::clone(&port));
        assert_eq!(
            run_broker_process(
                &cx,
                &spec("seat-a", "config-a"),
                b"input".to_vec(),
                &ProcessCancellation::default()
            )
            .unwrap(),
            b"ok"
        );
        let requests = port.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(
            request.argv.iter().map(ArgAtom::as_str).collect::<Vec<_>>(),
            ["spaces stay whole", "a'\"b"]
        );
        assert_eq!(request.root.as_str(), "seat-a");
        assert_eq!(request.environment.iter().count(), 1);
        assert_eq!(request.budget.stdin.as_deref(), Some(b"input".as_slice()));
    }

    #[test]
    fn missing_binding_refusal_timeout_cancel_and_output_bound_are_typed() {
        let cx = test_cx();
        assert!(
            active_process_port(&cx)
                .err()
                .unwrap()
                .to_string()
                .contains("no active process port")
        );
        let port = Arc::new(ModelPort::new(vec![
            ProcessAttempt::NotDispatched {
                refusal: ProcessRefusal::SpawnFailed("missing program".into()),
            },
            ProcessAttempt::StoppedAfterTimeout { receipt: stop() },
            completed("too much", 0, true),
        ]));
        let cx = cx_with(port);
        let request = spec("seat", "config");
        assert!(
            run_broker_process(&cx, &request, vec![], &ProcessCancellation::default())
                .unwrap_err()
                .to_string()
                .contains("missing program")
        );
        assert!(
            run_broker_process(&cx, &request, vec![], &ProcessCancellation::default())
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(
            run_broker_process(&cx, &request, vec![], &ProcessCancellation::default())
                .unwrap_err()
                .to_string()
                .contains("exceeded")
        );
        let cancelled = ProcessCancellation::default();
        cancelled.cancel();
        assert!(
            run_broker_process(&cx, &request, vec![], &cancelled)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }

    #[test]
    fn two_seats_keep_config_mounts_separate() {
        let port = Arc::new(ModelPort::new(vec![
            completed("a", 0, false),
            completed("b", 0, false),
        ]));
        let cx = cx_with(Arc::clone(&port));
        run_broker_process(
            &cx,
            &spec("root-a", "config-a"),
            vec![],
            &ProcessCancellation::default(),
        )
        .unwrap();
        run_broker_process(
            &cx,
            &spec("root-b", "config-b"),
            vec![],
            &ProcessCancellation::default(),
        )
        .unwrap();
        let requests = port.requests.lock().unwrap();
        assert_ne!(requests[0].root, requests[1].root);
        assert_ne!(requests[0].private_artifacts, requests[1].private_artifacts);
    }

    #[test]
    fn framing_is_bounded_validated_and_fail_closed() {
        assert_eq!(
            frame_stdout(b"{\"a\":1}\n{\"b\":2}\n".to_vec(), "json-lines")
                .unwrap()
                .len(),
            2
        );
        assert!(frame_stdout(b"not-json\n".to_vec(), "json-lines").is_err());
        assert!(frame_stdout(vec![], "invented").is_err());
        assert!(
            ArgAtom::new("bad\0argument").is_err(),
            "the delivered UTF-8 port rejects unrepresentable native argv"
        );
    }
}
