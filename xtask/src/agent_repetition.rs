use std::{fs, path::Path};

const ROOTS: &[&str] = &[
    "crates/sim-lib-agent/src",
    "crates/sim-lib-bridge/src",
    "crates/sim-lib-agent-runner-core/src",
    "crates/sim-lib-agent-runner-http/src",
    "crates/sim-lib-agent-runner-local/src",
    "crates/sim-lib-agent-runner-process/src",
];

const TRANSPORT_LOOPS: &[(&str, &str)] = &[(
    "crates/sim-lib-agent/src/components/runtime/process.rs",
    "bounded child-process pipe draining; no model decision or repair is repeated",
)];

const TOPOLOGY_SCHEDULERS: &[&str] = &[
    "crates/sim-lib-agent/src/agents/ops/topology_data.rs",
    "crates/sim-lib-agent/src/agents/ops/topology_helpers.rs",
    "crates/sim-lib-agent/src/agents/ops/topology_runtime.rs",
    "crates/sim-lib-agent/src/agents/ops/topology_sites.rs",
];

pub fn check(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for relative in ROOTS {
        collect_rs(&root.join(relative), &mut files)?;
    }
    let mut failures = Vec::new();
    for path in files {
        if path.components().any(|part| part.as_os_str() == "tests") {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let name = relative.to_string_lossy();
        let source = fs::read_to_string(&path).map_err(|error| format!("{name}: {error}"))?;
        let transport = TRANSPORT_LOOPS.iter().find(|(path, _)| *path == name);
        let topology_scheduler = TOPOLOGY_SCHEDULERS.iter().any(|path| *path == name);
        for (index, line) in source.lines().enumerate() {
            let normalized = line.to_ascii_lowercase();
            let owns_counter = ["attempt", "retry", "round", "turn"].iter().any(|word| {
                normalized.contains(word)
                    && (normalized.contains("let mut ")
                        || normalized.contains("+=")
                        || normalized.trim_start().starts_with("for "))
            });
            let owns_loop = normalized.contains("loop {")
                && ["model", "tool", "repair"]
                    .iter()
                    .any(|word| source.contains(word));
            if (owns_counter || owns_loop) && transport.is_none() && !topology_scheduler {
                failures.push(format!(
                    "{}:{} owns agent repetition outside the conduct scheduler: {}",
                    name,
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    if failures.is_empty() {
        for (path, reason) in TRANSPORT_LOOPS {
            if reason.trim().is_empty() || !root.join(path).is_file() {
                return Err(format!("invalid transport retry allowlist entry {path}"));
            }
        }
        println!(
            "agent repetition: conduct-owned; {} named transport loop contract(s)",
            TRANSPORT_LOOPS.len()
        );
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn collect_rs(path: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))? {
        let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_has_no_private_agent_repetition() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives directly below the repository root");
        check(root).expect("agent repetition inventory must remain conduct-owned");
    }
}
