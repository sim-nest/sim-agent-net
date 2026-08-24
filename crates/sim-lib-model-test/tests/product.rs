use std::fs;

use sim_lib_model_test::product::{BUNDLE_SCHEMA, OUTPUT_SCHEMA, run_command};

#[test]
fn no_network_observatory_replay_is_deterministic() {
    let path = std::env::temp_dir().join(format!("model-test-e2e-{}.json", std::process::id()));
    for verb in [
        "census",
        "packs",
        "run",
        "resume",
        "status",
        "verify",
        "report",
        "decide",
        "disposition",
        "pick",
    ] {
        let output = run_command(&["model-test".into(), verb.into(), "--machine".into()]).unwrap();
        assert!(output.contains(OUTPUT_SCHEMA));
        if !matches!(verb, "run" | "resume") {
            assert!(output.contains("\"provider_contact\":\"none\""));
        }
    }
    let args = [
        "model-test".into(),
        "export".into(),
        "--output".into(),
        path.display().to_string(),
    ];
    let output = run_command(&args).unwrap();
    assert!(output.contains(BUNDLE_SCHEMA));
    let before = fs::read(&path).unwrap();
    run_command(&[
        "model-test".into(),
        "export".into(),
        "--from".into(),
        path.display().to_string(),
        "--output".into(),
        path.display().to_string(),
    ])
    .unwrap();
    assert_eq!(before, fs::read(&path).unwrap());
    let _ = fs::remove_file(path);
}
