use super::*;
use crate::bind_process_port;
use sim_lib_exec::{
    ProcResult, ProcessAttempt, ProcessPort, ProcessReceipt, ProcessRequest, StopReceipt,
};
use sim_lib_provider::{OpenCodeTermsPolicy, TermsAcknowledgement};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Reply {
    Text(&'static str),
    Timeout,
    Death,
}
struct FixturePort {
    replies: Mutex<Vec<Reply>>,
    requests: Mutex<Vec<ProcessRequest>>,
}
impl FixturePort {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            requests: Mutex::default(),
        })
    }
}
impl ProcessPort for FixturePort {
    fn run(&self, request: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
        self.requests.lock().unwrap().push(request.clone());
        match self.replies.lock().unwrap().pop().unwrap() {
            Reply::Text(stdout) => ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "opencode-fixture".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout: stdout.into(),
                        stderr: String::new(),
                        exit_code: 0,
                        truncated: false,
                    },
                },
            },
            Reply::Death => ProcessAttempt::Completed {
                receipt: ProcessReceipt {
                    provider: "opencode-fixture".into(),
                    elapsed_mono_ns: 1,
                    result: ProcResult {
                        stdout: String::new(),
                        stderr: "died".into(),
                        exit_code: 9,
                        truncated: false,
                    },
                },
            },
            Reply::Timeout => ProcessAttempt::StoppedAfterTimeout {
                receipt: StopReceipt {
                    provider: "opencode-fixture".into(),
                    elapsed_mono_ns: 1,
                    cleanup: "reaped".into(),
                },
            },
        }
    }
}
fn cx(port: Arc<FixturePort>) -> Cx {
    let mut cx = Cx::new(
        Arc::new(sim_kernel::eval::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(0x4f50_454e),
    );
    bind_process_port(&mut cx, port).unwrap();
    cx
}
fn policy(allowed: bool, kind: AuthMethod) -> OpenCodeTermsPolicy {
    OpenCodeTermsPolicy {
        vendor: "vendor-a".into(),
        credential_kind: kind,
        use_allowed: allowed,
        terms_id: "vendor-a-terms".into(),
        revision: "2026-08".into(),
        acknowledgement: Some(TermsAcknowledgement {
            terms_id: "vendor-a-terms".into(),
            revision: "2026-08".into(),
        }),
    }
}
fn config(label: &str, transport: OpenCodeTransport) -> OpenCodeConfig {
    OpenCodeConfig {
        label: label.into(),
        artifact: format!("config-{label}"),
        expected_version: "opencode 1.2.3".into(),
        workspace: format!("workspace-{label}"),
        provider: "vendor-a".into(),
        model: "model-x".into(),
        agent: "build".into(),
        config_digest: format!("sha256:config-{label}"),
        plugin_digest: "sha256:plugins-a".into(),
        transport,
        terms_policy: policy(true, AuthMethod::ApiKey),
    }
}
fn adapter(configs: Vec<OpenCodeConfig>) -> OpenCodeCliAdapter {
    OpenCodeCliAdapter::new(
        ProgramRef::new("opencode").unwrap(),
        configs,
        Duration::from_millis(10),
        16_384,
    )
    .unwrap()
}

#[test]
fn each_config_home_and_declared_server_is_a_distinct_transport_honest_seat() {
    let port = FixturePort::new(vec![
        Reply::Text("opencode 1.2.3"),
        Reply::Text(r#"{"digest":"sha256:catalog-observation"}"#),
    ]);
    let seats = adapter(vec![
        config("process", OpenCodeTransport::Process),
        config(
            "server",
            OpenCodeTransport::LocalServer {
                endpoint: "http://127.0.0.1:4096".into(),
                password_ref: "secret:opencode-server".into(),
            },
        ),
    ])
    .discover(&mut cx(port), Expr::Nil)
    .unwrap();
    assert_eq!(
        seats.iter().map(|s| s.seat.to_string()).collect::<Vec<_>>(),
        ["seat:opencode-cli#process", "seat:opencode-cli#server"]
    );
    assert_eq!(seats[0].endpoint.transport, Symbol::new("local-process"));
    assert_eq!(seats[1].endpoint.transport, Symbol::new("local-server"));
    let harness = seats[0].harness.as_ref().unwrap();
    for key in [
        "provider-selection",
        "model-selection",
        "agent-selection",
        "config-digest",
        "plugin-digest",
        "transport-kind",
        "observed-catalog-digest",
    ] {
        assert!(
            harness
                .extra
                .iter()
                .any(|(k, _)| k == &Expr::Symbol(Symbol::new(key)))
        );
    }
    assert!(
        seats[0]
            .extra
            .iter()
            .any(|(_, v)| v == &Expr::String("observed-only".into()))
    );
}

#[test]
fn raw_and_framed_events_decode_while_malformed_and_incomplete_output_fail_closed() {
    let raw = "{\"type\":\"text\",\"text\":\"raw\"}\n{\"type\":\"done\"}";
    let framed = r#"[{"type":"text","text":"framed"},{"type":"done"}]"#;
    assert_eq!(
        decode_events(raw, "m").unwrap().content,
        vec![Expr::String("raw".into())]
    );
    assert_eq!(
        decode_events(framed, "m").unwrap().content,
        vec![Expr::String("framed".into())]
    );
    assert!(
        decode_events("{broken", "m")
            .unwrap_err()
            .to_string()
            .contains("malformed")
    );
    assert!(
        decode_events(r#"{"type":"text","text":"partial"}"#, "m")
            .unwrap_err()
            .to_string()
            .contains("before done")
    );
}

#[test]
fn provider_model_plugin_policy_and_transport_drift_are_pinned_before_inference() {
    let port = FixturePort::new(vec![
        Reply::Text("opencode 1.2.3"),
        Reply::Text(r#"{"digest":"sha256:catalog"}"#),
    ]);
    let mut runtime = cx(port);
    let adapter = adapter(vec![config("main", OpenCodeTransport::Process)]);
    let mut seat = adapter.discover(&mut runtime, Expr::Nil).unwrap().remove(0);
    seat.endpoint.transport = Symbol::new("local-server");
    assert!(
        adapter
            .open(&mut runtime, &seat, Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("implicit fallback")
    );

    let mut forbidden = config("subscription", OpenCodeTransport::Process);
    forbidden.terms_policy = policy(false, AuthMethod::Subscription);
    let silent = FixturePort::new(Vec::new());
    assert!(
        adapter(vec![forbidden])
            .discover(&mut cx(Arc::clone(&silent)), Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("forbidden through OpenCode")
    );
    assert!(
        silent.requests.lock().unwrap().is_empty(),
        "terms policy must run before transport"
    );
}

#[test]
fn timeout_process_death_server_death_and_removal_are_isolated() {
    for (reply, expected) in [(Reply::Timeout, "timed out"), (Reply::Death, "status 9")] {
        let adapter = adapter(vec![config("process", OpenCodeTransport::Process)]);
        assert!(
            adapter
                .discover(&mut cx(FixturePort::new(vec![reply])), Expr::Nil)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
    }
    let server = config(
        "server",
        OpenCodeTransport::LocalServer {
            endpoint: "http://127.0.0.1:9".into(),
            password_ref: "secret:server".into(),
        },
    );
    let mut runtime = cx(FixturePort::new(Vec::new()));
    let adapter = adapter(vec![server]);
    let seat = adapter.discover(&mut runtime, Expr::Nil).unwrap().remove(0);
    assert!(
        adapter
            .open(&mut runtime, &seat, Expr::Nil)
            .unwrap_err()
            .to_string()
            .contains("process fallback is forbidden")
    );

    let mut registry = ProviderRegistry::new();
    register_opencode_cli(&mut registry, adapter).unwrap();
    assert_eq!(registry.families().len(), 1);
    let empty = ProviderRegistry::new();
    assert!(
        empty.families().is_empty(),
        "the shared registry has no OpenCode catalog or credential dependency"
    );
}
