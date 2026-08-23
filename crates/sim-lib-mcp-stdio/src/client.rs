use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use sim_lib_agent_runner_process::{ProcessProgram, StderrSink};
use sim_lib_exec::{ProcessCancellation, ProcessPort};

use crate::{FrameError, JsonLineFramer};

/// Process-client construction bounds.
#[derive(Clone, Debug)]
pub struct ProcessClientOptions {
    /// Wire line bound.
    pub max_frame_bytes: usize,
    /// Per-exchange deadline (must not exceed the program budget).
    pub deadline: Duration,
    /// Probe modern discovery before ordinary calls.
    pub discovery_probe: bool,
}

impl Default for ProcessClientOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024,
            deadline: Duration::from_secs(30),
            discovery_probe: true,
        }
    }
}

/// Deterministic process-peer failure.
#[derive(Debug)]
pub enum ProcessClientError {
    /// Framing failure.
    Frame(FrameError),
    /// Process execution or death.
    Process(String),
    /// No response matched the independent request id.
    MissingResponse,
    /// Child exited unsuccessfully.
    Died(i32),
}
impl fmt::Display for ProcessClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(e) => write!(f, "{e}"),
            Self::Process(e) => f.write_str(e),
            Self::MissingResponse => f.write_str("process ended without matching MCP response"),
            Self::Died(code) => write!(f, "MCP process died with status {code}"),
        }
    }
}
impl std::error::Error for ProcessClientError {}

/// MCP client peer executed only through a structured [`ProcessProgram`].
pub struct McpProcessClient {
    program: ProcessProgram,
    options: ProcessClientOptions,
    next_id: AtomicU64,
}

impl McpProcessClient {
    /// Constructs a peer with an independent id namespace.
    pub fn new(
        program: ProcessProgram,
        options: ProcessClientOptions,
    ) -> Result<Self, ProcessClientError> {
        JsonLineFramer::new(options.max_frame_bytes).map_err(ProcessClientError::Frame)?;
        if options.deadline.is_zero() {
            return Err(ProcessClientError::Process(
                "deadline must be non-zero".into(),
            ));
        }
        let configured_deadline = Duration::from_millis(program.request().budget.timeout_ms);
        if options.deadline != configured_deadline {
            return Err(ProcessClientError::Process(format!(
                "client deadline {:?} must equal the ProcessProgram budget {:?}",
                options.deadline, configured_deadline
            )));
        }
        Ok(Self {
            program,
            options,
            next_id: AtomicU64::new(1),
        })
    }
    /// Runs the modern discovery probe when enabled.
    pub fn discover(
        &self,
        port: &dyn ProcessPort,
        stderr: &mut dyn StderrSink,
        cancellation: &ProcessCancellation,
    ) -> Result<Option<Value>, ProcessClientError> {
        if !self.options.discovery_probe {
            return Ok(None);
        }
        self.request(port, stderr, cancellation, "server/discover", json!({}))
            .map(Some)
    }
    /// Sends one independently identified request, drains stderr, maps death,
    /// and returns only the matching response object.
    pub fn request(
        &self,
        port: &dyn ProcessPort,
        stderr: &mut dyn StderrSink,
        cancellation: &ProcessCancellation,
        method: &str,
        params: Value,
    ) -> Result<Value, ProcessClientError> {
        if cancellation.is_cancelled() {
            return Err(ProcessClientError::Process("request cancelled".into()));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let framer =
            JsonLineFramer::new(self.options.max_frame_bytes).map_err(ProcessClientError::Frame)?;
        let mut input = Vec::new();
        framer
            .write(&mut input, &request)
            .map_err(ProcessClientError::Frame)?;
        let mut output = Vec::new();
        let report = self
            .program
            .exchange(
                port,
                [input],
                &mut |chunk: &[u8]| {
                    output.extend_from_slice(chunk);
                    Ok(())
                },
                stderr,
                cancellation,
            )
            .map_err(|e| ProcessClientError::Process(e.to_string()))?;
        if report.exit_code != 0 {
            return Err(ProcessClientError::Died(report.exit_code));
        }
        let mut cursor = std::io::Cursor::new(output);
        while let Some(value) = framer
            .read(&mut cursor)
            .map_err(ProcessClientError::Frame)?
        {
            if value.get("id") == Some(&json!(id)) {
                return Ok(value);
            }
        }
        Err(ProcessClientError::MissingResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::Result as SimResult;
    use sim_lib_exec::{
        ProcResult, ProcessAttempt, ProcessBudget, ProcessReceipt, ProcessRequest, ProgramRef,
        ProjectRootRef, SealedBindings,
    };
    use std::sync::Mutex;

    struct Peer {
        exit_code: i32,
        ids: Mutex<Vec<u64>>,
    }
    impl ProcessPort for Peer {
        fn run(&self, request: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
            let wire: Value =
                serde_json::from_slice(request.budget.stdin.as_ref().unwrap()).unwrap();
            let id = wire["id"].as_u64().unwrap();
            self.ids.lock().unwrap().push(id);
            ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "test-peer".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout: format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{}}}}\n"),
                        stderr: "bounded diagnostic\n".into(),
                        exit_code: self.exit_code,
                        truncated: false,
                    },
                },
            }
        }
    }
    #[derive(Default)]
    struct Stderr(Vec<u8>);
    impl StderrSink for Stderr {
        fn write_stderr(&mut self, chunk: &[u8]) -> SimResult<()> {
            self.0.extend_from_slice(chunk);
            Ok(())
        }
    }
    fn client() -> McpProcessClient {
        let request = ProcessRequest {
            program: ProgramRef::new("mcp-peer").unwrap(),
            argv: Vec::new(),
            root: ProjectRootRef::new("project").unwrap(),
            environment: SealedBindings::default(),
            private_artifacts: Vec::new(),
            budget: ProcessBudget {
                timeout_ms: 100,
                max_output_bytes: 4096,
                stdin: None,
            },
        };
        McpProcessClient::new(
            ProcessProgram::new(request, 4096).unwrap(),
            ProcessClientOptions {
                max_frame_bytes: 4096,
                deadline: Duration::from_millis(100),
                discovery_probe: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn probes_with_independent_ids_and_drains_stderr() {
        let client = client();
        let peer = Peer {
            exit_code: 0,
            ids: Mutex::new(Vec::new()),
        };
        let mut stderr = Stderr::default();
        client
            .discover(&peer, &mut stderr, &ProcessCancellation::default())
            .unwrap();
        client
            .request(
                &peer,
                &mut stderr,
                &ProcessCancellation::default(),
                "tools/list",
                json!({}),
            )
            .unwrap();
        assert_eq!(*peer.ids.lock().unwrap(), [1, 2]);
        assert_eq!(stderr.0, b"bounded diagnostic\nbounded diagnostic\n");
    }

    #[test]
    fn maps_cancellation_death_and_deadline_mismatch() {
        let client = client();
        let cancelled = ProcessCancellation::default();
        cancelled.cancel();
        assert!(matches!(
            client.request(
                &Peer {
                    exit_code: 0,
                    ids: Mutex::new(Vec::new())
                },
                &mut Stderr::default(),
                &cancelled,
                "x",
                json!({})
            ),
            Err(ProcessClientError::Process(_))
        ));
        assert!(matches!(
            client.request(
                &Peer {
                    exit_code: 9,
                    ids: Mutex::new(Vec::new())
                },
                &mut Stderr::default(),
                &ProcessCancellation::default(),
                "x",
                json!({})
            ),
            Err(ProcessClientError::Died(9))
        ));
        let mut request = client.program.request().clone();
        request.budget.stdin = None;
        assert!(
            McpProcessClient::new(
                ProcessProgram::new(request, 4096).unwrap(),
                ProcessClientOptions {
                    deadline: Duration::from_millis(99),
                    ..ProcessClientOptions::default()
                }
            )
            .is_err()
        );
    }
}
