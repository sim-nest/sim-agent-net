use std::fs;
use std::path::{Path, PathBuf};

use crate::recipe_assertions::check_repo;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sim-agent-net-recipe-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_recipe_at(root: &Path, id: &str, body: &str) {
    let dir = root.join("crates/sim-lib-agent/recipes/30-agents").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("recipe.toml"), body).unwrap();
    fs::write(dir.join("setup.siml"), "(quote ok)").unwrap();
    fs::write(dir.join("purpose.md"), "Synthetic decision.").unwrap();
    fs::write(dir.join("expected.txt"), "ok\n").unwrap();
}

fn write_recipe(root: &Path, body: &str) {
    write_recipe_at(root, "a30-001-autonomous-decision", body);
}

fn valid_recipe() -> &'static str {
    r#"
id = "a30-001-autonomous-decision"
title = "Autonomous decision"
codec = "lisp"
setup = "setup.siml"
purpose = "purpose.md"
order = 1
tags = ["30-agents", "chapter-05", "autonomous-decision", "offline"]
requires = ["agent", "codec/lisp"]
recipe_number = 1
source_chapter = 5
architecture_family = "autonomous-decision"
runner_mode = "fake"
safety_posture = "offline"
capabilities = ["read-eval"]
allow_capabilities = ["read-eval"]
descriptor_shape = "agent-decision"
assert_tags = ["30-agents", "chapter-05", "autonomous-decision", "offline"]
assert_capabilities = ["read-eval"]
assert_allow_capabilities = ["read-eval"]
assert_setup_codec = "lisp"
assert_descriptor_shape = "agent-decision"
expected = "expected.txt"
[[expect]]
form = 0
result = "ok"
"#
}

fn capstone_recipe() -> &'static str {
    r#"
id = "a30-capstone-domain-transforming-integration"
title = "Domain-transforming integration capstone"
codec = "lisp"
setup = "setup.siml"
purpose = "purpose.md"
order = 130
tags = ["30-agents", "chapter-16", "domain-transforming-integration", "offline", "capstone", "outside-30-count"]
requires = ["agent", "codec/lisp"]
source_chapter = 16
architecture_family = "domain-transforming-integration"
runner_mode = "fake"
safety_posture = "offline"
capabilities = ["read-eval"]
allow_capabilities = ["read-eval"]
descriptor_shape = "domain-transforming-integration-capstone"
assert_tags = ["30-agents", "chapter-16", "capstone", "outside-30-count"]
assert_capabilities = ["read-eval"]
assert_allow_capabilities = ["read-eval"]
assert_setup_codec = "lisp"
assert_descriptor_shape = "domain-transforming-integration-capstone"
expected = "expected.txt"
[[expect]]
form = 0
result = "ok"
"#
}

#[test]
fn checks_agent30_recipe_metadata_and_assertions() {
    let root = temp_root("ok");
    write_recipe(&root, valid_recipe());
    let summary = check_repo(&root).unwrap();
    assert_eq!(summary.checked_recipes, 1);
    assert_eq!(summary.agent30_recipes, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn checks_capstone_without_counting_it_as_numbered_recipe() {
    let root = temp_root("capstone");
    write_recipe_at(
        &root,
        "a30-capstone-domain-transforming-integration",
        capstone_recipe(),
    );
    let summary = check_repo(&root).unwrap();
    assert_eq!(summary.checked_recipes, 1);
    assert_eq!(summary.agent30_recipes, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reports_capstone_recipe_number() {
    let root = temp_root("capstone-number");
    write_recipe_at(
        &root,
        "a30-capstone-domain-transforming-integration",
        &capstone_recipe().replace(
            "source_chapter = 16\n",
            "recipe_number = 31\nsource_chapter = 16\n",
        ),
    );
    let err = check_repo(&root).unwrap_err();
    assert!(
        err.contains("capstone recipes outside the 30 count must omit `recipe_number`"),
        "{err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reports_missing_agent30_metadata() {
    let root = temp_root("missing");
    write_recipe(
        &root,
        &valid_recipe().replace("architecture_family = \"autonomous-decision\"\n", ""),
    );
    let err = check_repo(&root).unwrap_err();
    assert!(err.contains("missing `architecture_family`"), "{err}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reports_assertion_mismatch() {
    let root = temp_root("assert");
    write_recipe(
        &root,
        &valid_recipe().replace(
            "assert_tags = [\"30-agents\", \"chapter-05\", \"autonomous-decision\", \"offline\"]",
            "assert_tags = [\"missing-tag\"]",
        ),
    );
    let err = check_repo(&root).unwrap_err();
    assert!(err.contains("missing asserted tag `missing-tag`"), "{err}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reports_allow_capability_assertion_mismatch() {
    let root = temp_root("allow-cap");
    write_recipe(
        &root,
        &valid_recipe().replace(
            "assert_allow_capabilities = [\"read-eval\"]",
            "assert_allow_capabilities = [\"missing-cap\"]",
        ),
    );
    let err = check_repo(&root).unwrap_err();
    assert!(
        err.contains("missing asserted allowed capability `missing-cap`"),
        "{err}"
    );
    let _ = fs::remove_dir_all(root);
}
