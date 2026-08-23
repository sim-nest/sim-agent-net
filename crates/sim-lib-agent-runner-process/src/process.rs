use sim_kernel::{ClassRef, Cx, Error, Object, ObjectCompat, Result, Symbol};
use sim_lib_exec::{
    ArgAtom, PrivateArtifactRef, ProcessAttempt, ProcessBudget, ProcessCancellation, ProcessPort,
    ProcessRequest, ProgramRef, ProjectRootRef, SealedBindings,
};
use std::{any::Any, sync::Arc, time::Duration};

// conformance: structured process programs preserve literal argv and bounded duplex framing.

/// Bounded stderr consumer for a structured process exchange.
pub trait StderrSink {
    /// Accepts one stderr chunk. Returning an error cancels the exchange.
    fn write_stderr(&mut self, chunk: &[u8]) -> Result<()>;
}

/// Structured terminal report for one broker child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessExitReport {
    /// Native exit status, or the portable provider sentinel.
    pub exit_code: i32,
    /// Capsule identity from the process receipt.
    pub provider: String,
    /// Provider-measured elapsed monotonic time.
    pub elapsed_mono_ns: u64,
}

/// MCP-oriented duplex projection over a delivered [`ProcessRequest`].
///
/// Input chunks are bounded and joined only up to the request's shared output cap. Output and
/// stderr are emitted incrementally in fixed-size frames after the sole [`ProcessPort`] dispatch.
/// The platform adapter remains responsible for concurrent pipe draining and child reaping.
#[derive(Clone, Debug)]
pub struct ProcessProgram {
    request: ProcessRequest,
    frame_bytes: usize,
}

impl ProcessProgram {
    /// Creates a structured program from the exact portable process request.
    pub fn new(request: ProcessRequest, frame_bytes: usize) -> Result<Self> {
        if frame_bytes == 0 || request.budget.max_output_bytes == 0 {
            return Err(Error::Eval(
                "process framing and output caps must be non-zero".into(),
            ));
        }
        Ok(Self {
            request,
            frame_bytes,
        })
    }
    /// Returns the literal request delivered to the process capsule.
    #[must_use]
    pub fn request(&self) -> &ProcessRequest {
        &self.request
    }
    /// Runs through `ProcessPort`, framing both output directions without shell interpretation.
    pub fn exchange<I, O>(
        &self,
        port: &dyn ProcessPort,
        stdin: I,
        stdout: &mut O,
        stderr: &mut dyn StderrSink,
        cancellation: &ProcessCancellation,
    ) -> Result<ProcessExitReport>
    where
        I: IntoIterator<Item = Vec<u8>>,
        O: FnMut(&[u8]) -> Result<()>,
    {
        let mut request = self.request.clone();
        let mut input = Vec::new();
        for chunk in stdin {
            if cancellation.is_cancelled() {
                return Err(Error::Eval("process exchange cancelled".into()));
            }
            if chunk.is_empty()
                || chunk.len() > self.frame_bytes
                || input.len().saturating_add(chunk.len()) > request.budget.max_output_bytes
            {
                return Err(Error::Eval(
                    "process stdin exceeded its bounded framing contract".into(),
                ));
            }
            input.extend_from_slice(&chunk);
        }
        request.budget.stdin = Some(input);
        let receipt = match port.run(&request, cancellation) {
            ProcessAttempt::Completed { receipt } => receipt,
            ProcessAttempt::NotDispatched { refusal } => {
                return Err(Error::HostError(format!(
                    "process refused before dispatch: {refusal:?}"
                )));
            }
            ProcessAttempt::StoppedAfterTimeout { .. } => {
                return Err(Error::Eval("process exchange timed out".into()));
            }
            ProcessAttempt::StoppedAfterCancel { .. } => {
                return Err(Error::Eval("process exchange cancelled".into()));
            }
            ProcessAttempt::UnknownAfterDispatch { evidence } => {
                return Err(Error::HostError(format!(
                    "process outcome unknown after {}: {}",
                    evidence.stage, evidence.detail
                )));
            }
        };
        let captured_bytes = receipt
            .result
            .stdout
            .len()
            .saturating_add(receipt.result.stderr.len());
        if receipt.result.truncated || captured_bytes > request.budget.max_output_bytes {
            return Err(Error::Eval("process output cap exceeded".into()));
        }
        for chunk in receipt.result.stdout.as_bytes().chunks(self.frame_bytes) {
            stdout(chunk)?;
        }
        for chunk in receipt.result.stderr.as_bytes().chunks(self.frame_bytes) {
            stderr.write_stderr(chunk)?;
        }
        Ok(ProcessExitReport {
            exit_code: receipt.result.exit_code,
            provider: receipt.provider,
            elapsed_mono_ns: receipt.elapsed_mono_ns,
        })
    }
}

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

    #[derive(Default)]
    struct CapturedStderr(Vec<Vec<u8>>);
    impl StderrSink for CapturedStderr {
        fn write_stderr(&mut self, chunk: &[u8]) -> Result<()> {
            self.0.push(chunk.to_vec());
            Ok(())
        }
    }

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

    #[test]
    fn process_program_delivers_hostile_argv_literally_without_shell_reparsing() {
        let spec = BrokerProcessSpec::new(
            ProgramRef::new("mcp-fixture").unwrap(),
            vec![ArgAtom::new("$(touch /tmp/nope); echo pwned").unwrap()],
            ProjectRootRef::new("fixture-root").unwrap(),
            SealedBindings::empty(),
            vec![],
            "mcp",
            Duration::from_secs(1),
            64,
        )
        .unwrap();
        let port = ModelPort::new(vec![ProcessAttempt::Completed {
            receipt: ProcessReceipt {
                provider: "provider-4".into(),
                elapsed_mono_ns: 7,
                result: ProcResult {
                    stdout: "{}\n".into(),
                    stderr: "warn".into(),
                    exit_code: 0,
                    truncated: false,
                },
            },
        }]);
        let program = ProcessProgram::new(spec.request().clone(), 4).unwrap();
        let mut output = Vec::new();
        let mut errors = CapturedStderr::default();
        let report = program
            .exchange(
                &port,
                vec![b"{}\n".to_vec()],
                &mut |chunk| {
                    output.push(chunk.to_vec());
                    Ok(())
                },
                &mut errors,
                &ProcessCancellation::default(),
            )
            .unwrap();
        let requests = port.requests.lock().unwrap();
        assert_eq!(
            requests[0].argv[0].as_str(),
            "$(touch /tmp/nope); echo pwned"
        );
        assert_eq!(
            requests[0].budget.stdin.as_deref(),
            Some(b"{}\n".as_slice())
        );
        assert_eq!(output, vec![b"{}\n".to_vec()]);
        assert_eq!(errors.0, vec![b"warn".to_vec()]);
        assert_eq!(report.exit_code, 0);
    }

    #[test]
    fn process_program_rejects_stdin_and_stderr_floods_at_fixed_caps() {
        let program = ProcessProgram::new(spec("seat", "config").request().clone(), 4).unwrap();
        let port = ModelPort::new(vec![ProcessAttempt::Completed {
            receipt: ProcessReceipt {
                provider: "provider-4".into(),
                elapsed_mono_ns: 7,
                result: ProcResult {
                    stdout: String::new(),
                    stderr: "x".repeat(65),
                    exit_code: 0,
                    truncated: false,
                },
            },
        }]);
        let mut errors = CapturedStderr::default();
        assert!(
            program
                .exchange(
                    &port,
                    vec![b"12345".to_vec()],
                    &mut |_| Ok(()),
                    &mut errors,
                    &ProcessCancellation::default()
                )
                .unwrap_err()
                .to_string()
                .contains("stdin")
        );
        assert!(
            program
                .exchange(
                    &port,
                    Vec::<Vec<u8>>::new(),
                    &mut |_| Ok(()),
                    &mut errors,
                    &ProcessCancellation::default()
                )
                .unwrap_err()
                .to_string()
                .contains("output cap")
        );
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
            sim_kernel::HandleSeed::new(0x5052_4f43),
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
