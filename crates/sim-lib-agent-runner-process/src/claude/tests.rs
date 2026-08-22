use super::*;
use crate::bind_process_port;
use sim_lib_exec::{
    ProcResult, ProcessAttempt, ProcessPort, ProcessReceipt, ProcessRequest, StopReceipt,
};
use sim_lib_provider::{ClaudeCliTermsPolicy, TermsAcknowledgement};
use std::sync::{Arc, Barrier, Mutex};

#[derive(Clone)]
enum Reply {
    Text(String),
    Timeout,
}

struct FixturePort {
    replies: Mutex<Vec<Reply>>,
    requests: Mutex<Vec<ProcessRequest>>,
}

impl FixturePort {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
            ),
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
                    provider: "claude-fixture".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout,
                        stderr: String::new(),
                        exit_code: 0,
                        truncated: false,
                    },
                },
            },
            Reply::Timeout => ProcessAttempt::StoppedAfterTimeout {
                receipt: StopReceipt {
                    provider: "claude-fixture".into(),
                    elapsed_mono_ns: 1,
                    cleanup: "reaped".into(),
                },
            },
        }
    }
}

fn allowed_policy() -> ClaudeCliTermsPolicy {
    ClaudeCliTermsPolicy {
        subscription_use_allowed: true,
        terms_id: "anthropic-commercial-terms".into(),
        revision: "2026-08-01".into(),
        acknowledgement: Some(TermsAcknowledgement {
            terms_id: "anthropic-commercial-terms".into(),
            revision: "2026-08-01".into(),
            acknowledged_by: "operator-fixture".into(),
        }),
    }
}

fn home(label: &str) -> ClaudeCliConfigHome {
    ClaudeCliConfigHome {
        label: label.into(),
        artifact: format!("claude-home-{label}"),
        expected_version: "2.1.0 (Claude Code)".into(),
        config_home_digest: format!("sha256:home-{label}"),
        visible_settings_digest: format!("sha256:settings-{label}"),
        model: "claude-opus-4-1".into(),
        permission_mode: "default".into(),
        max_turns: 8,
        max_budget_usd: Some("1.50".into()),
        terms_policy: allowed_policy(),
    }
}

fn help() -> String {
    "--print --output-format stream-json --model --permission-mode --max-turns --max-budget-usd"
        .into()
}

fn auth(status: &str, extra: &str) -> String {
    format!(r#"{{"schema":"claude-stream-json/1","status":"{status}"{extra}}}"#)
}

fn probe_replies(status: &str, extra: &str) -> Vec<Reply> {
    vec![
        Reply::Text("2.1.0 (Claude Code)".into()),
        Reply::Text(help()),
        Reply::Text(auth(status, extra)),
    ]
}

fn cx(port: Arc<FixturePort>) -> Cx {
    let mut cx = Cx::new(
        Arc::new(sim_kernel::eval::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(0x434c_4155),
    );
    bind_process_port(&mut cx, port).unwrap();
    cx
}

fn adapter(homes: Vec<ClaudeCliConfigHome>) -> ClaudeCliAdapter {
    ClaudeCliAdapter::new(
        ProgramRef::new("claude").unwrap(),
        homes,
        Duration::from_millis(10),
        16_384,
    )
    .unwrap()
}

#[test]
fn subscription_status_and_two_config_homes_are_distinct_complete_seats() {
    let replies = probe_replies("authenticated", r#", "principal":"personal""#)
        .into_iter()
        .chain(probe_replies("authenticated", r#", "principal":"work""#))
        .collect::<Vec<_>>();
    let port = FixturePort::new(replies);
    let mut cx = cx(Arc::clone(&port));
    let seats = adapter(vec![home("personal"), home("work")])
        .discover(&mut cx, Expr::Nil)
        .unwrap();
    assert_eq!(
        seats
            .iter()
            .map(|seat| seat.seat.to_string())
            .collect::<Vec<_>>(),
        ["seat:claude-cli#personal", "seat:claude-cli#work"]
    );
    assert!(
        seats
            .iter()
            .all(|seat| seat.family == Symbol::qualified("provider", "claude-cli"))
    );
    let harness = seats[0].harness.as_ref().unwrap();
    for key in [
        "config-home-digest",
        "selected-model",
        "visible-settings-digest",
        "permission-mode",
    ] {
        assert!(
            harness
                .extra
                .iter()
                .any(|(candidate, _)| candidate == &Expr::Symbol(Symbol::new(key)))
        );
    }
    let requests = port.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(
        requests
            .iter()
            .all(|request| format!("{:?}", request.environment).contains("CLAUDE_CONFIG_DIR"))
    );
}

#[test]
fn login_required_browser_handoff_expired_token_and_unknown_schema_are_typed_or_refused() {
    for (status, extra, expected) in [
        ("login-required", "", SessionStatus::LoginRequired),
        (
            "browser-handoff",
            r#", "url":"https://login.example/claude""#,
            SessionStatus::BrowserHandoff {
                url: "https://login.example/claude".into(),
            },
        ),
        ("expired-token", "", SessionStatus::LoginRequired),
    ] {
        let port = FixturePort::new(probe_replies(status, extra));
        let mut cx = cx(port);
        let seat = adapter(vec![home("seat")])
            .discover(&mut cx, Expr::Nil)
            .unwrap()
            .remove(0);
        assert_eq!(seat.auth_metadata().unwrap().unwrap().session, expected);
    }

    let port = FixturePort::new([
        Reply::Text("2.1.0 (Claude Code)".into()),
        Reply::Text(help()),
        Reply::Text(r#"{"schema":"claude-stream-json/99","status":"authenticated"}"#.into()),
    ]);
    assert!(
        adapter(vec![home("seat")])
            .discover(&mut cx(port), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("unknown output schema")
    );
}

#[test]
fn missing_machine_mode_version_drift_malformed_output_timeout_and_quota_fail_closed() {
    let missing = FixturePort::new([
        Reply::Text("2.1.0 (Claude Code)".into()),
        Reply::Text("--print --model".into()),
    ]);
    assert!(
        adapter(vec![home("seat")])
            .discover(&mut cx(missing), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("required flag")
    );
    let drift = FixturePort::new([Reply::Text("3.0.0 (Claude Code)".into())]);
    assert!(
        adapter(vec![home("seat")])
            .discover(&mut cx(drift), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("version drifted")
    );
    let malformed = FixturePort::new([
        Reply::Text("2.1.0 (Claude Code)".into()),
        Reply::Text(help()),
        Reply::Text("{broken".into()),
    ]);
    assert!(
        adapter(vec![home("seat")])
            .discover(&mut cx(malformed), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("malformed auth output")
    );
    let timeout = FixturePort::new([Reply::Timeout]);
    assert!(
        adapter(vec![home("seat")])
            .discover(&mut cx(timeout), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(decode_stream_json(r#"{"schema":"claude-stream-json/1","type":"result","subtype":"error","error":"quota exhausted"}"#, "claude").unwrap_err().to_string().contains("quota exhausted"));
    assert!(
        decode_stream_json("{broken", "claude")
            .unwrap_err()
            .to_string()
            .contains("malformed stream-json")
    );
}

#[test]
fn forbidden_or_unacknowledged_terms_refuse_before_process_connection() {
    let port = FixturePort::new(Vec::<Reply>::new());
    let mut forbidden = home("forbidden");
    forbidden.terms_policy.subscription_use_allowed = false;
    let mut cx = cx(Arc::clone(&port));
    assert!(
        adapter(vec![forbidden])
            .discover(&mut cx, Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("forbidden")
    );
    assert!(port.requests.lock().unwrap().is_empty());

    let mut unacknowledged = home("unacknowledged");
    unacknowledged.terms_policy.acknowledgement = None;
    assert!(
        adapter(vec![unacknowledged])
            .discover(&mut cx, Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("must be acknowledged")
    );
    assert!(port.requests.lock().unwrap().is_empty());
}

#[test]
fn two_claude_homes_answer_in_one_fanout_without_identity_collapse() {
    let barrier = Arc::new(Barrier::new(3));
    let mut joins = Vec::new();
    for label in ["personal", "work"] {
        let barrier = Arc::clone(&barrier);
        joins.push(std::thread::spawn(move || {
            let output = concat!(
                "{\"schema\":\"claude-stream-json/1\",\"type\":\"assistant\",\"text\":\"same-answer\"}\n",
                "{\"schema\":\"claude-stream-json/1\",\"type\":\"result\",\"subtype\":\"success\"}\n"
            );
            let port = FixturePort::new([Reply::Text(output.into())]);
            let mut cx = cx(port);
            let config = home(label);
            let card = config.seat_card(&ClaudeCliProbe {
                version: config.expected_version.clone(), machine_mode: "print-stream-json".into(),
                output_schema: "claude-stream-json/1".into(),
                session: SessionStatus::Authenticated { principal_label: Some(label.into()) },
                principal_label: Some(label.into()),
            }).unwrap();
            let runner = adapter(vec![config]).open(&mut cx, &card, Expr::Nil).unwrap();
            barrier.wait();
            (card.seat.to_string(), runner.infer(&mut cx, ModelRequest::default()).unwrap().content)
        }));
    }
    barrier.wait();
    let rows = joins
        .into_iter()
        .map(|join| join.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter()
            .map(|(seat, _)| seat.as_str())
            .collect::<Vec<_>>(),
        ["seat:claude-cli#personal", "seat:claude-cli#work"]
    );
    assert!(
        rows.iter()
            .all(|(_, content)| *content == vec![Expr::String("same-answer".into())])
    );
}

#[test]
fn claude_cli_and_anthropic_api_answer_concurrently_with_distinct_guidance() {
    let barrier = Arc::new(Barrier::new(2));
    let claude_barrier = Arc::clone(&barrier);
    let claude = std::thread::spawn(move || {
        let output = concat!(
            "{\"schema\":\"claude-stream-json/1\",\"type\":\"assistant\",\"text\":\"answer\"}\n",
            "{\"schema\":\"claude-stream-json/1\",\"type\":\"result\",\"subtype\":\"success\"}\n"
        );
        let port = FixturePort::new([Reply::Text(output.into())]);
        let mut cx = cx(port);
        let config = home("subscription");
        let card = config
            .seat_card(&ClaudeCliProbe {
                version: config.expected_version.clone(),
                machine_mode: "print-stream-json".into(),
                output_schema: "claude-stream-json/1".into(),
                session: SessionStatus::Authenticated {
                    principal_label: Some("subscription".into()),
                },
                principal_label: Some("subscription".into()),
            })
            .unwrap();
        let runner = adapter(vec![config])
            .open(&mut cx, &card, Expr::Nil)
            .unwrap();
        claude_barrier.wait();
        (
            card.family.to_string(),
            card.seat.to_string(),
            "vendor CLI login; inspect Claude subscription/quota",
            runner
                .infer(&mut cx, ModelRequest::default())
                .unwrap()
                .content,
        )
    });
    barrier.wait();
    let anthropic = (
        "provider/anthropic-api",
        "provider/anthropic-api:api-prod",
        "rotate SIM secret-provider API key; inspect HTTP quota",
        vec![Expr::String("answer".into())],
    );
    let claude = claude.join().unwrap();
    assert_ne!(claude.0, anthropic.0);
    assert_ne!(claude.1, anthropic.1);
    assert_ne!(claude.2, anthropic.2);
    assert_eq!(claude.3, anthropic.3);
}
