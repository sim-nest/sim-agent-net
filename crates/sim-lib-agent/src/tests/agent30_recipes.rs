use std::{fs, path::PathBuf};

const EXPECTED_RECIPE_IDS: [&str; 30] = [
    "a30-001-autonomous-decision",
    "a30-002-planning",
    "a30-003-memory-augmented",
    "a30-004-knowledge-retrieval",
    "a30-005-document-intelligence",
    "a30-006-scientific-research",
    "a30-007-tool-using",
    "a30-008-chain-orchestrator",
    "a30-009-agentic-workflow",
    "a30-010-data-analysis",
    "a30-011-verification-validation",
    "a30-012-general-problem-solver",
    "a30-013-code-generation",
    "a30-014-security-hardened",
    "a30-015-self-improving",
    "a30-016-conversational",
    "a30-017-content-creation",
    "a30-018-recommendation",
    "a30-019-vision-language",
    "a30-020-audio-processing",
    "a30-021-physical-sensing",
    "a30-022-ethical-reasoning",
    "a30-023-explainable",
    "a30-024-healthcare-intelligence",
    "a30-025-scientific-discovery",
    "a30-026-financial-advisory",
    "a30-027-legal-intelligence",
    "a30-028-education-intelligence",
    "a30-029-collective-intelligence",
    "a30-030-embodied-intelligence",
];

const EXPECTED_CAPSTONE_RECIPE_IDS: [&str; 2] = [
    "a30-capstone-domain-transforming-integration",
    "a30-capstone-agent-society",
];

const EXPECTED_ATELIER_RECIPE_IDS: [&str; 9] = [
    "atelier-radar-standard-crate",
    "atelier-runtime-operation",
    "atelier-codec-roundtrip",
    "atelier-guideline-firewall",
    "atelier-change-capsule",
    "atelier-contract-deck-assembly",
    "atelier-shape-query-cache",
    "atelier-contract-native-success",
    "atelier-cheap-first-escalation",
];

const EXPECTED_CHAPTER_TAGS: [&str; 12] = [
    "chapter-05",
    "chapter-06",
    "chapter-07",
    "chapter-08",
    "chapter-09",
    "chapter-10",
    "chapter-11",
    "chapter-12",
    "chapter-13",
    "chapter-14",
    "chapter-15",
    "chapter-16",
];

const EXPECTED_ARCHITECTURE_TAGS: [&str; 30] = [
    "autonomous-decision",
    "planning",
    "memory-augmented",
    "knowledge-retrieval",
    "document-intelligence",
    "scientific-research",
    "tool-using",
    "chain-orchestrator",
    "agentic-workflow",
    "data-analysis",
    "verification-validation",
    "general-problem-solver",
    "code-generation",
    "security-hardened",
    "self-improving",
    "conversational",
    "content-creation",
    "recommendation",
    "vision-language",
    "audio-processing",
    "physical-sensing",
    "ethical-reasoning",
    "explainable",
    "healthcare-intelligence",
    "scientific-discovery",
    "financial-advisory",
    "legal-intelligence",
    "education-intelligence",
    "collective-intelligence",
    "embodied-intelligence",
];

#[test]
fn agent30_recipe_scaffold_records_stable_ids_without_placeholder_dirs() {
    let recipes = recipe_root();
    let book = fs::read_to_string(recipes.join("book.toml")).unwrap();
    assert!(
        book.contains("chapters = [\"01-basics\", \"30-agents\", \"40-atelier\", \"50-conducts\"]"),
        "{book}"
    );

    let chapter_dir = recipes.join("30-agents");
    let chapter = fs::read_to_string(chapter_dir.join("chapter.toml")).unwrap();
    assert!(chapter.contains("tags = [\"30-agents\""));
    for tag in EXPECTED_CHAPTER_TAGS {
        assert!(chapter.contains(tag), "missing {tag}");
    }
    assert!(chapter.contains("architecture_family_tags = ["));
    for tag in EXPECTED_ARCHITECTURE_TAGS {
        assert!(chapter.contains(tag), "missing architecture tag {tag}");
    }
    for recipe_id in EXPECTED_RECIPE_IDS {
        assert!(chapter.contains(recipe_id), "missing {recipe_id}");
    }
    assert!(chapter.contains("capstone_recipe_ids = ["));
    for recipe_id in EXPECTED_CAPSTONE_RECIPE_IDS {
        assert!(chapter.contains(recipe_id), "missing capstone {recipe_id}");
    }
    assert_eq!(
        chapter.matches("a30-").count(),
        EXPECTED_RECIPE_IDS.len() * 2 + EXPECTED_CAPSTONE_RECIPE_IDS.len()
    );

    let placeholder_dirs = fs::read_dir(chapter_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let dir = entry.path();
            !dir.join("recipe.toml").is_file()
                || !dir.join("setup.siml").is_file()
                || !dir.join("purpose.md").is_file()
                || !dir.join("expected.txt").is_file()
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(placeholder_dirs.is_empty(), "{placeholder_dirs:?}");

    let atelier_dir = recipes.join("40-atelier");
    let atelier = fs::read_to_string(atelier_dir.join("chapter.toml")).unwrap();
    assert!(atelier.contains("tags = [\"atelier\""));
    for recipe_id in EXPECTED_ATELIER_RECIPE_IDS {
        assert!(atelier.contains(recipe_id), "missing {recipe_id}");
        let recipe_dir = atelier_dir.join(recipe_id);
        assert!(recipe_dir.join("recipe.toml").is_file());
        assert!(recipe_dir.join("setup.siml").is_file());
        assert!(recipe_dir.join("purpose.md").is_file());
        assert!(recipe_dir.join("expected.txt").is_file());
    }
}

fn recipe_root() -> PathBuf {
    let direct = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("recipes");
    if direct.join("book.toml").is_file() {
        return direct;
    }

    let cwd = std::env::current_dir().unwrap();
    for ancestor in cwd.ancestors() {
        for candidate in [
            ancestor.join("crates/sim-lib-agent/recipes"),
            ancestor.join("../sim-agent-net/crates/sim-lib-agent/recipes"),
            ancestor
                .parent()
                .map(|parent| parent.join("sim-agent-net/crates/sim-lib-agent/recipes"))
                .unwrap_or_default(),
        ] {
            if candidate.join("book.toml").is_file() {
                return candidate;
            }
        }
    }

    panic!(
        "sim-lib-agent recipes directory not found from {}",
        cwd.display()
    );
}
