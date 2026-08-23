use std::fs;
use std::path::PathBuf;

#[test]
fn modern_default_dependency_closure_excludes_connection_and_transport_crates() {
    let manifest =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[build-dependencies]")
        .next()
        .unwrap();
    for forbidden in [
        "sim-lib-mcp-legacy",
        "sim-lib-agent-runner-core",
        "sim-lib-server",
        "sim-lib-stream-core",
        "sim-lib-stream-fabric",
        "sim-run-core",
    ] {
        let line = dependencies
            .lines()
            .find(|line| line.starts_with(forbidden))
            .unwrap_or("");
        assert!(
            line.is_empty() || line.contains("optional = true"),
            "modern default closure includes forbidden dependency: {line}"
        );
    }
    let default = manifest
        .lines()
        .find(|line| line.starts_with("default ="))
        .unwrap();
    assert_eq!(default, "default = []");
}
