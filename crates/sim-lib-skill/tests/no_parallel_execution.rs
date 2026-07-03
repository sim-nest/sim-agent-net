use std::{fs, path::Path};

#[test]
fn source_tree_has_no_parallel_skill_or_mcp_execution_families() {
    let root = workspace_root();
    let banned = [
        concat!("Skill", "Tool"),
        concat!("Adapter", "Tool"),
        concat!("Gateway", "Skill", "Registry"),
        concat!("Mcp", "Tool", "Registry"),
        concat!("Mcp", "Tool", "Executor"),
        concat!("Mcp", "Resource", "Registry"),
        concat!("Mcp", "Prompt", "Registry"),
        concat!("Mcp", "Eval", "Loop"),
        concat!("Mcp", "Progress", "Buffer"),
        concat!("Mcp", "Stream", "Queue"),
        concat!("Mcp", "Progress", "Queue"),
        concat!("Mcp", "Http", "Stack"),
        concat!("Mcp", "Http", "Server"),
        concat!("Mcp", "Sse", "Server"),
        concat!("Mcp", "Web", "Socket", "Server"),
        concat!("Skill", "Http", "Registry"),
        concat!("Http", "Skill", "Registry"),
        concat!("Skill", "Http", "Executor"),
        concat!("Skill", "Process", "Registry"),
        concat!("Process", "Skill", "Registry"),
        concat!("Skill", "Process", "Executor"),
        concat!("Process", "Skill", "Executor"),
        concat!("Skill", "Cache", "Registry"),
        concat!("Skill", "Cassette", "Registry"),
        concat!("Skill", "Audit", "Registry"),
        concat!("Skill", "Cache", "Executor"),
        concat!("Skill", "Cassette", "Executor"),
        concat!("Mcp", "Skill", "Callable"),
    ];
    let mut hits = Vec::new();

    for source_root in ["crates", "src"] {
        let path = root.join(source_root);
        if path.exists() {
            scan_for_banned_names(&path, &banned, &mut hits);
        }
    }

    assert!(
        hits.is_empty(),
        "parallel skill/MCP execution family names found:\n{}",
        hits.join("\n")
    );
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("sim-lib-skill should live under crates/")
}

fn scan_for_banned_names(path: &Path, banned: &[&str], hits: &mut Vec<String>) {
    let metadata = fs::metadata(path)
        .unwrap_or_else(|err| panic!("failed to read metadata for {}: {err}", path.display()));
    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        {
            let entry = entry
                .unwrap_or_else(|err| panic!("failed to read entry in {}: {err}", path.display()));
            scan_for_banned_names(&entry.path(), banned, hits);
        }
        return;
    }

    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    for (line_index, line) in contents.lines().enumerate() {
        for name in banned {
            if line.contains(name) {
                hits.push(format!("{}:{}:{name}", path.display(), line_index + 1));
            }
        }
    }
}
