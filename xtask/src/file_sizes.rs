use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const GENERAL_SOFT_LIMIT: usize = 500;
const GENERAL_HARD_LIMIT: usize = 700;
const ENTRYPOINT_SOFT_LIMIT: usize = 150;
const ENTRYPOINT_HARD_LIMIT: usize = 250;

const SOFT_EXEMPTIONS: &[SoftExemption] = &[
    SoftExemption {
        path: "crates/sim-lib-agent-runner-http/src/probe.rs",
        reason: "provider probe matrix keeps shared timeout, transport, and profile fixtures together",
    },
    SoftExemption {
        path: "crates/sim-lib-agent-runner-http/src/runner.rs",
        reason: "runner state machine keeps request, stream, and retry paths in one contract file",
    },
    SoftExemption {
        path: "crates/sim-lib-agent-runner-process/src/process.rs",
        reason: "process runner owns spawn, stdout, stderr, timeout, and streaming cleanup paths",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/components/market_execution.rs",
        reason: "market execution keeps race, speculate, debate, and escalation accounting together",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/components/placement.rs",
        reason: "model placement wires catalog registration, lookup, and cards through one surface",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/components/runtime/runner_tools.rs",
        reason: "runner tool loop keeps descriptor filtering and execution boundary checks together",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/fairness.rs",
        reason: "fairness facets share one vocabulary and scoring test surface",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/lib.rs",
        reason: "entrypoint re-exports a broad loadable agent surface while keeping behavior in modules",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/tests/agent_ai_agent_runner.rs",
        reason: "agent-as-runner tests share one setup for model endpoint projection cases",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/tests/agent_ai_placement.rs",
        reason: "placement tests share catalog fixtures across direct, cached, and routed cases",
    },
    SoftExemption {
        path: "crates/sim-lib-agent/src/tests/agent_ai_tool_injection.rs",
        reason: "tool injection tests keep allow, deny, conflict, and trace fixtures side by side",
    },
    SoftExemption {
        path: "crates/sim-lib-cookbook/src/tests.rs",
        reason: "cookbook integration tests share one eval-capable runtime harness",
    },
    SoftExemption {
        path: "crates/sim-lib-forge/src/lift_frontier.rs",
        reason: "frontier lift logic keeps row validation and repair-menu construction together",
    },
    SoftExemption {
        path: "crates/sim-lib-forge/src/tests.rs",
        reason: "forge tests share compiled-intent fixtures across lift, verify, route, and cache cases",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/objects.rs",
        reason: "gateway object records stay colocated to keep JSON transcript shape invariants visible",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/lib.rs",
        reason: "entrypoint re-exports the gateway API surface while route behavior stays in modules",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/routes/replay.rs",
        reason: "replay and fork routes share request lineage and capability checks",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/storage/objects.rs",
        reason: "object storage keeps in-memory indexing and response content invariants together",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/tests.rs",
        reason: "gateway integration tests share route setup and stored-object fixtures",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/tests/plan.rs",
        reason: "plan tests keep parser, shape, and eval fixture coverage together",
    },
    SoftExemption {
        path: "crates/sim-lib-openai-server/src/translate/tools.rs",
        reason: "tool translation keeps OpenAI schema mapping and capability checks in one boundary",
    },
    SoftExemption {
        path: "crates/sim-lib-server/src/lib.rs",
        reason: "entrypoint re-exports the server API surface while behavior stays in modules",
    },
    SoftExemption {
        path: "crates/sim-lib-server/src/server.rs",
        reason: "server lifecycle keeps status, runtime wiring, and request dispatch together",
    },
    SoftExemption {
        path: "crates/sim-lib-server/src/tests/cookbook_web.rs",
        reason: "cookbook web tests share HTTP route and recipe-store fixtures",
    },
    SoftExemption {
        path: "crates/sim-lib-server/src/transport/tests/http.rs",
        reason: "transport tests share HTTP, SSE, WebSocket, and connection fixtures",
    },
    SoftExemption {
        path: "crates/sim-lib-skill/src/tests.rs",
        reason: "skill integration tests share card, policy, browse, and projection fixtures",
    },
    SoftExemption {
        path: "crates/sim-lib-stream-fabric/src/placement.rs",
        reason: "stream placement keeps rank, cassette, and remote realization rules together",
    },
    SoftExemption {
        path: "crates/sim-lib-stream-fabric/src/tests.rs",
        reason: "stream fabric tests share packet, frame, cassette, and placement fixtures",
    },
    SoftExemption {
        path: "xtask/src/recipe_assertions.rs",
        reason: "recipe assertions keep the local manifest parser and policy checks together",
    },
];

pub fn run(args: &[String]) -> Result<(), String> {
    let program = args.first().map(String::as_str).unwrap_or("xtask");
    if args.len() != 2 {
        return Err(format!("usage: {program} check-file-sizes"));
    }

    validate_exemptions()?;
    let root = std::env::current_dir().map_err(|err| format!("current dir: {err}"))?;
    let summary = scan_root(&root)?;
    if summary.hard_failures == 0 && summary.soft_failures == 0 {
        println!(
            "check-file-sizes: OK ({} Rust file(s), {} soft exemption(s), 0 soft failure(s), 0 hard failure(s))",
            summary.files, summary.soft_exemptions
        );
        Ok(())
    } else {
        Err(format!(
            "check-file-sizes: FAILED ({} Rust file(s), {} soft failure(s), {} hard failure(s))",
            summary.files, summary.soft_failures, summary.hard_failures
        ))
    }
}

fn scan_root(root: &Path) -> Result<ScanSummary, String> {
    let mut paths = Vec::new();
    collect_rs_files(root, &mut paths)?;
    paths.sort();

    let mut seen_exemptions = BTreeSet::new();
    let mut summary = ScanSummary::default();
    for path in paths {
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let line_count = text.lines().count();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let rel = slash(relative);
        match classify(relative, line_count) {
            FileSizeStatus::Ok => {}
            FileSizeStatus::Soft { limit } => {
                if let Some(exemption) = exemption_for(&rel) {
                    seen_exemptions.insert(exemption.path);
                    summary.soft_exemptions += 1;
                    eprintln!(
                        "note: {rel} has {line_count} lines (soft target {limit}); exempted: {}",
                        exemption.reason
                    );
                } else {
                    summary.soft_failures += 1;
                    eprintln!("error: {rel} has {line_count} lines (soft target {limit})");
                }
            }
            FileSizeStatus::Hard { limit } => {
                summary.hard_failures += 1;
                eprintln!("error: {rel} has {line_count} lines (hard limit {limit})");
            }
        }
        summary.files += 1;
    }

    for exemption in SOFT_EXEMPTIONS {
        if !seen_exemptions.contains(exemption.path) {
            return Err(format!(
                "check-file-sizes: stale soft exemption for {}",
                exemption.path
            ));
        }
    }

    Ok(summary)
}

fn validate_exemptions() -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for exemption in SOFT_EXEMPTIONS {
        if exemption.reason.trim().is_empty() {
            return Err(format!(
                "check-file-sizes: {} has an empty exemption reason",
                exemption.path
            ));
        }
        if !paths.insert(exemption.path) {
            return Err(format!(
                "check-file-sizes: duplicate soft exemption for {}",
                exemption.path
            ));
        }
    }
    Ok(())
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if should_skip_dir(dir) {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(|err| format!("read dir {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read dir entry {}: {err}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if file_type.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension() == Some(OsStr::new("rs")) {
            out.push(path);
        }
    }

    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some(".git" | "target" | ".meta-workspace" | "sim-tooling")
    )
}

fn classify(path: &Path, line_count: usize) -> FileSizeStatus {
    let limits = limits_for(path);
    if line_count > limits.hard {
        FileSizeStatus::Hard { limit: limits.hard }
    } else if line_count > limits.soft {
        FileSizeStatus::Soft { limit: limits.soft }
    } else {
        FileSizeStatus::Ok
    }
}

fn limits_for(path: &Path) -> Limits {
    match path.file_name().and_then(OsStr::to_str) {
        Some("lib.rs" | "main.rs" | "mod.rs") => Limits {
            soft: ENTRYPOINT_SOFT_LIMIT,
            hard: ENTRYPOINT_HARD_LIMIT,
        },
        _ => Limits {
            soft: GENERAL_SOFT_LIMIT,
            hard: GENERAL_HARD_LIMIT,
        },
    }
}

fn exemption_for(path: &str) -> Option<&'static SoftExemption> {
    SOFT_EXEMPTIONS
        .iter()
        .find(|exemption| exemption.path == path)
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Copy)]
struct SoftExemption {
    path: &'static str,
    reason: &'static str,
}

#[derive(Default)]
struct ScanSummary {
    files: usize,
    soft_exemptions: usize,
    soft_failures: usize,
    hard_failures: usize,
}

#[derive(Clone, Copy)]
struct Limits {
    soft: usize,
    hard: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileSizeStatus {
    Ok,
    Soft { limit: usize },
    Hard { limit: usize },
}

#[cfg(test)]
mod tests {
    use super::{FileSizeStatus, classify, limits_for, should_skip_dir};
    use std::path::Path;

    #[test]
    fn entrypoint_limits_are_stricter() {
        assert_eq!(limits_for(Path::new("src/lib.rs")).soft, 150);
        assert_eq!(limits_for(Path::new("src/lib.rs")).hard, 250);
        assert_eq!(limits_for(Path::new("src/logic.rs")).soft, 500);
        assert_eq!(limits_for(Path::new("src/logic.rs")).hard, 700);
    }

    #[test]
    fn line_counts_classify_soft_and_hard_limits() {
        assert_eq!(classify(Path::new("src/logic.rs"), 500), FileSizeStatus::Ok);
        assert_eq!(
            classify(Path::new("src/logic.rs"), 501),
            FileSizeStatus::Soft { limit: 500 }
        );
        assert_eq!(
            classify(Path::new("src/logic.rs"), 701),
            FileSizeStatus::Hard { limit: 700 }
        );
        assert_eq!(
            classify(Path::new("src/main.rs"), 251),
            FileSizeStatus::Hard { limit: 250 }
        );
    }

    #[test]
    fn skips_ci_tooling_checkout() {
        assert!(should_skip_dir(Path::new("sim-tooling")));
        assert!(should_skip_dir(Path::new("nested/sim-tooling")));
    }
}
