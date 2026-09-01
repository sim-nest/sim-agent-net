use super::*;
#[test]
fn api_subscription_cli_and_local_daemon_are_distinct_command_rows() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/provider-seats.toml");
    let output = run(&[
        "provider".into(),
        "--inventory".into(),
        path.display().to_string(),
        "seats".into(),
    ])
    .unwrap();
    assert_eq!(output.lines().count(), 3);
    for id in [
        "openai-api-primary",
        "codex-subscription-primary",
        "ollama-local",
    ] {
        assert!(output.contains(&format!("seat-id={id} ")));
    }
    assert!(output.contains("principal-kind=api-key"));
    assert!(output.contains("principal-kind=subscription"));
    assert!(output.contains("principal-kind=none"));
    let fanout = run(&[
        "provider".into(),
        "--inventory".into(),
        path.display().to_string(),
        "fan-out".into(),
        "all".into(),
    ])
    .unwrap();
    assert_eq!(fanout.lines().count(), 3);
}
