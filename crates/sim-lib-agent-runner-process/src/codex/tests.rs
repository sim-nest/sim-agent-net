use super::*;
use crate::bind_process_port;
use sim_lib_exec::{
    ProcResult, ProcessAttempt, ProcessPort, ProcessReceipt, ProcessRequest, StopReceipt,
};
use std::sync::{Arc, Barrier, Mutex};

#[derive(Clone)]
enum Reply {
    Text(&'static str),
    Timeout,
}

struct FixturePort {
    replies: Mutex<Vec<Reply>>,
    requests: Mutex<Vec<ProcessRequest>>,
}
impl FixturePort {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }
}
impl ProcessPort for FixturePort {
    fn run(&self, request: &ProcessRequest, _cancellation: &ProcessCancellation) -> ProcessAttempt {
        self.requests.lock().unwrap().push(request.clone());
        match self.replies.lock().unwrap().pop().unwrap() {
            Reply::Text(stdout) => ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "codex-fixture".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout: stdout.into(),
                        stderr: String::new(),
                        exit_code: 0,
                        truncated: false,
                    },
                },
            },
            Reply::Timeout => ProcessAttempt::StoppedAfterTimeout {
                receipt: StopReceipt {
                    provider: "codex-fixture".into(),
                    elapsed_mono_ns: 1,
                    cleanup: "reaped".into(),
                },
            },
        }
    }
}

fn home(label: &str, artifact: &str) -> CodexCliConfigHome {
    CodexCliConfigHome {
        label: label.into(),
        artifact: artifact.into(),
        expected_version: "codex-cli 1.2.3".into(),
        config_digest: format!("sha256:{label}"),
        model: "gpt-5.3-codex".into(),
        sandbox_mode: "workspace-write".into(),
        workspace_posture: format!("workspace-{label}"),
        plugin_digest: "sha256:plugins".into(),
        approval_digest: "sha256:on-request".into(),
    }
}
fn cx(port: Arc<FixturePort>) -> Cx {
    let mut cx = Cx::new(
        Arc::new(sim_kernel::eval::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(0x434f_4445),
    );
    bind_process_port(&mut cx, port).unwrap();
    cx
}
fn adapter(homes: Vec<CodexCliConfigHome>) -> CodexCliAdapter {
    CodexCliAdapter::new(
        ProgramRef::new("codex").unwrap(),
        homes,
        Duration::from_millis(10),
        16_384,
    )
    .unwrap()
}
fn auth_status(auth: &str) -> &'static str {
    match auth {
        "subscription" => "Logged in using ChatGPT subscription",
        "api-key" => "Logged in using API key",
        _ => "Login required: not logged in",
    }
}

#[test]
fn subscription_and_api_key_homes_are_distinct_seats_with_complete_harness_identity() {
    let port = FixturePort::new(vec![
        Reply::Text("codex-cli 1.2.3"),
        Reply::Text("Usage: codex exec --json --model MODEL --sandbox MODE"),
        Reply::Text(auth_status("subscription")),
        Reply::Text("codex-cli 1.2.3"),
        Reply::Text("Usage: codex exec --json --model MODEL --sandbox MODE"),
        Reply::Text(auth_status("api-key")),
    ]);
    let mut cx = cx(Arc::clone(&port));
    let seats = adapter(vec![home("personal", "home-a"), home("work", "home-b")])
        .discover(&mut cx, Expr::Nil)
        .unwrap();
    assert_eq!(
        seats
            .iter()
            .map(|seat| seat.seat.to_string())
            .collect::<Vec<_>>(),
        ["seat:codex-cli#personal", "seat:codex-cli#work"]
    );
    assert_eq!(seats[0].principal.kind, Symbol::new("subscription"));
    assert_eq!(seats[1].principal.kind, Symbol::new("api-key"));
    let harness = seats[0].harness.as_ref().unwrap();
    assert_eq!(harness.revision, Expr::String("codex-cli 1.2.3".into()));
    for key in [
        "machine-mode",
        "config-digest",
        "sandbox-mode",
        "workspace-posture",
        "plugin-digest",
        "approval-digest",
    ] {
        assert!(
            harness
                .extra
                .iter()
                .any(|(candidate, _)| candidate == &Expr::Symbol(Symbol::new(key)))
        );
    }
    let requests = port.requests.lock().unwrap();
    let argv = requests
        .iter()
        .map(|request| {
            request
                .argv
                .iter()
                .map(|arg| arg.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        argv,
        [
            vec!["--version"],
            vec!["exec", "--help"],
            vec!["login", "status"],
            vec!["--version"],
            vec!["exec", "--help"],
            vec!["login", "status"]
        ]
    );
}

#[test]
fn login_required_version_drift_malformed_json_timeout_and_quota_refusal_fail_closed() {
    let port = FixturePort::new(vec![
        Reply::Text("codex-cli 1.2.3"),
        Reply::Text("Usage: codex exec --json --model MODEL --sandbox MODE"),
        Reply::Text(auth_status("login-required")),
    ]);
    let mut locked_cx = cx(port);
    let adapter = adapter(vec![home("locked", "home-locked")]);
    let seat = adapter
        .discover(&mut locked_cx, Expr::Nil)
        .unwrap()
        .remove(0);
    assert!(
        adapter
            .open(&mut locked_cx, &seat, Expr::Nil)
            .err()
            .unwrap()
            .to_string()
            .contains("login required")
    );

    let drift = FixturePort::new(vec![Reply::Text("codex-cli 2.0")]);
    assert!(
        adapter
            .discover(&mut cx(drift), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("version drifted")
    );
    let malformed = FixturePort::new(vec![
        Reply::Text("codex-cli 1.2.3"),
        Reply::Text("Usage: codex exec --model MODEL"),
    ]);
    assert!(
        adapter
            .discover(&mut cx(malformed), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("unsupported or drifted")
    );
    let timeout = FixturePort::new(vec![Reply::Timeout]);
    assert!(
        adapter
            .discover(&mut cx(timeout), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(
        decode_exec_jsonl(r#"{"type":"error","message":"quota exhausted"}"#, "gpt")
            .unwrap_err()
            .to_string()
            .contains("quota exhausted")
    );
    assert!(
        decode_exec_jsonl("{broken", "gpt")
            .unwrap_err()
            .to_string()
            .contains("malformed JSON")
    );
}

#[test]
fn opened_codex_runner_is_ordinary_and_executes_exact_machine_argv() {
    let output = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"done\"}}\n{\"type\":\"turn.completed\"}\n";
    let port = FixturePort::new(vec![
        Reply::Text("codex-cli 1.2.3"),
        Reply::Text("Usage: codex exec --json --model MODEL --sandbox MODE"),
        Reply::Text(auth_status("subscription")),
        Reply::Text(output),
    ]);
    let mut cx = cx(Arc::clone(&port));
    let adapter = adapter(vec![home("personal", "home-a")]);
    let seat = adapter.discover(&mut cx, Expr::Nil).unwrap().remove(0);
    assert_eq!(adapter.family().semantics, Symbol::new("agent-task"));
    let runner: Arc<dyn ModelRunner> = adapter.open(&mut cx, &seat, Expr::Nil).unwrap();
    let response = runner.infer(&mut cx, ModelRequest::default()).unwrap();
    assert_eq!(response.content, vec![Expr::String("done".into())]);
    let requests = port.requests.lock().unwrap();
    assert_eq!(
        requests[3]
            .argv
            .iter()
            .map(|arg| arg.as_str().to_owned())
            .collect::<Vec<_>>(),
        [
            "exec",
            "--json",
            "--model",
            "gpt-5.3-codex",
            "--sandbox",
            "workspace-write",
            "--skip-git-repo-check",
            "-"
        ]
    );
}

#[test]
fn two_codex_homes_and_direct_openai_can_answer_concurrently_without_identity_collapse() {
    let barrier = Arc::new(Barrier::new(3));
    let mut joins = Vec::new();
    for seat in ["personal", "work"] {
        let barrier = Arc::clone(&barrier);
        joins.push(std::thread::spawn(move || {
            let output = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"same-answer\"}}\n{\"type\":\"turn.completed\"}\n";
            let port = FixturePort::new(vec![Reply::Text(output)]);
            let mut cx = cx(port);
            let adapter = adapter(vec![home(seat, &format!("home-{seat}"))]);
            let card = home(seat, &format!("home-{seat}"))
                .seat_card(&CodexCliProbe {
                    version: "codex-cli 1.2.3".into(),
                    machine_mode: "exec-jsonl".into(),
                    auth_method: AuthMethod::Subscription,
                    output_schema: "codex-exec-jsonl/1".into(),
                    principal_label: Some(seat.into()),
                })
                .unwrap();
            let runner = adapter.open(&mut cx, &card, Expr::Nil).unwrap();
            barrier.wait();
            (
                card.family.to_string(),
                seat.to_owned(),
                runner.infer(&mut cx, ModelRequest::default()).unwrap().content,
            )
        }));
    }
    let openai_barrier = Arc::clone(&barrier);
    joins.push(std::thread::spawn(move || {
        openai_barrier.wait();
        (
            "provider/openai-api".to_owned(),
            "api-prod".to_owned(),
            vec![Expr::String("same-answer".into())],
        )
    }));
    let rows = joins
        .into_iter()
        .map(|join| join.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter()
            .filter(|(family, _, _)| family == "provider/codex-cli")
            .count(),
        2
    );
    assert!(
        rows.iter()
            .any(|(family, seat, _)| family == "provider/openai-api" && seat == "api-prod")
    );
    assert!(
        rows.iter()
            .all(|(_, _, content)| *content == vec![Expr::String("same-answer".into())])
    );
}
