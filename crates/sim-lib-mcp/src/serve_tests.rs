use super::*;

#[test]
fn serve_lib_exports_cli_main_mcp() {
    let manifest = McpServeLib::new().manifest();
    assert!(manifest.exports.iter().any(|export| matches!(
        export,
        Export::Function { symbol, .. } if symbol == &mcp_serve_entrypoint_symbol()
    )));
}

#[test]
fn parses_stdio_profile_caps_and_filters() {
    let opts = CliOptions::parse_from([
        "--stdio".to_owned(),
        "--profile".to_owned(),
        "default".to_owned(),
        "--allow-tool".to_owned(),
        "core.*".to_owned(),
        "--deny-tool".to_owned(),
        "*.danger*".to_owned(),
        "--cap".to_owned(),
        "mcp.tools.call".to_owned(),
        "--log-stderr".to_owned(),
    ])
    .unwrap();

    assert_eq!(opts.transport, Transport::Stdio);
    assert_eq!(
        opts.capabilities,
        vec![CapabilityName::new("mcp.tools.call")]
    );
    assert!(opts.log_stderr);
    assert!(opts.profile.allows_name("core.echo"));
    assert!(!opts.profile.allows_name("core.dangerous"));
    assert_eq!(opts.max_body_bytes, 1024 * 1024);
}

#[test]
fn duplicate_transport_is_rejected() {
    let err = CliOptions::parse_from(["--stdio".to_owned(), "--http".to_owned(), "x:1".to_owned()])
        .unwrap_err();
    assert!(format!("{err}").contains("one transport"));
}

#[test]
fn http_security_is_explicit_and_fail_closed() {
    let error = CliOptions::parse_from(["--http".into(), "0.0.0.0:8080".into()]).unwrap_err();
    assert!(error.to_string().contains("requires exact origin"));
    let options = CliOptions::parse_from([
        "--http".into(),
        "127.0.0.1:0".into(),
        "--anonymous-loopback".into(),
    ])
    .unwrap();
    assert!(options.anonymous_loopback);
}
