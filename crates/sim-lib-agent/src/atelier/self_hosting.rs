//! Deterministic self-hosting scenario fixtures for the Atelier.

use sim_cookbook::fnv1a64_hex;

/// One self-hosting scenario exercised by recipes and conformance tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfHostingScenario {
    /// Stable scenario id, matching the recipe directory.
    pub id: &'static str,
    /// Scenario title.
    pub title: &'static str,
    /// Runner mode used by the scenario.
    pub runner_mode: &'static str,
    /// Whether a live model is used.
    pub live_model: bool,
    /// Whether network access is used.
    pub network: bool,
    /// Required evidence families.
    pub evidence: &'static [&'static str],
    /// Roles participating in the scenario.
    pub roles: &'static [&'static str],
    /// Cassette event payloads used for the replay hash.
    pub cassette_events: &'static [&'static str],
    /// Expected replay hash for `cassette_events`.
    pub cassette_hash: &'static str,
}

/// Returns the Atelier self-hosting scenarios used by recipe and cassette tests.
pub fn self_hosting_scenarios() -> Vec<SelfHostingScenario> {
    vec![
        SelfHostingScenario {
            id: "atelier-radar-standard-crate",
            title: "Retrieval Radar standard crate",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["radar", "ranked-hints", "confidence"],
            roles: &["cartographer"],
            cassette_events: &[
                "fake-runner:load-index:sim-kernel",
                "radar-query:standard-crate-operations",
                "rank:hints:3",
                "explain:confidence:0.91",
            ],
            cassette_hash: "fnv1a64:7f9f5b6c744aa528",
        },
        SelfHostingScenario {
            id: "atelier-runtime-operation",
            title: "Runtime operation edit loop",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "runtime-operation",
                "rustdoc",
                "codec-prism",
                "generated-docs",
                "validation",
                "pin-plan",
            ],
            roles: &["editor", "validator", "docs-agent", "pin-agent"],
            cassette_events: &[
                "fake-runner:open-runtime-op-descriptor",
                "editor:add-operation:sample-op",
                "codec-prism:lisp-json-roundtrip",
                "validate:meta-check",
                "pin-plan:sim-runtime",
            ],
            cassette_hash: "fnv1a64:8a3282f3c7ef34b2",
        },
        SelfHostingScenario {
            id: "atelier-codec-roundtrip",
            title: "Codec round-trip fixture",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["codec-prism", "roundtrip", "semantic-id", "simdoc"],
            roles: &["editor", "validator"],
            cassette_events: &[
                "fake-runner:load-codec-fixture",
                "codec-prism:lisp-json-algol-roundtrip",
                "assert:semantic-id-stable",
                "docs:simdoc-check",
            ],
            cassette_hash: "fnv1a64:32be093d6f32315a",
        },
        SelfHostingScenario {
            id: "atelier-guideline-firewall",
            title: "Guideline Firewall bad fixture",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["guideline-firewall", "rule-evidence", "refused-capability"],
            roles: &["guard", "reviewer"],
            cassette_events: &[
                "fake-runner:load-bad-fixture",
                "guard:present-tense-public-docs",
                "refuse:EditRepo(sim-web)",
                "evidence:past-tense-wording",
            ],
            cassette_hash: "fnv1a64:768ad342812bdd9a",
        },
        SelfHostingScenario {
            id: "atelier-change-capsule",
            title: "Multi-agent Change Capsule",
            runner_mode: "cassette",
            live_model: false,
            network: false,
            evidence: &[
                "change-capsule",
                "validation",
                "docs",
                "pin-plan",
                "human-gate",
                "replay-hash",
            ],
            roles: &[
                "cartographer",
                "editor",
                "guard",
                "validator",
                "docs-agent",
                "pin-agent",
                "reviewer",
                "human-gate",
            ],
            cassette_events: &[
                "cassette-runner:cartographer-scope",
                "fake-runner:editor-patch",
                "fake-runner:guard-approve",
                "process:validator-pass",
                "process:docs-agent-pass",
                "fake-runner:pin-agent-plan",
                "fake-runner:reviewer-summary",
                "human-gate:approved",
                "replay:hash-match",
            ],
            cassette_hash: "fnv1a64:5ec7c4222478f8f1",
        },
        SelfHostingScenario {
            id: "atelier-contract-deck-assembly",
            title: "Contract deck assembly",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "contract-deck",
                "contract-card-shape",
                "shape-synthesized-example",
                "partial-card-diagnostics",
            ],
            roles: &["cartographer", "contract-author"],
            cassette_events: &[
                "contract-deck:assemble:registry-generation-1",
                "contract-card:shape:data-roundtrip",
                "contract-card:example:shape-synthesized",
                "contract-gap:partial-card-preserved",
            ],
            cassette_hash: "fnv1a64:f016c320c6277c50",
        },
        SelfHostingScenario {
            id: "atelier-shape-query-cache",
            title: "Cached ShapeQuery retrieval",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "shape-query",
                "contract-deck-cache",
                "cache-hit",
                "shape-specificity",
            ],
            roles: &["cartographer"],
            cassette_events: &[
                "shape-query:table-set-entries",
                "contract-deck-cache:miss:generation-1",
                "contract-deck-cache:hit:generation-1",
                "rank:shape-specificity:2",
            ],
            cassette_hash: "fnv1a64:ffca094bf5b5fb1e",
        },
        SelfHostingScenario {
            id: "atelier-contract-native-success",
            title: "Contract-native authoring success",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "contract-native",
                "shapegrammar",
                "shape-check",
                "diminished-realize",
                "checked-form",
                "cassette",
            ],
            roles: &["contract-author", "validator"],
            cassette_events: &[
                "contract-native:author:strict-grammar",
                "contract-native:decode:codec-lisp",
                "contract-native:shape-check:accepted",
                "contract-native:realize:diminished",
                "contract-native:cassette:accepted",
            ],
            cassette_hash: "fnv1a64:099488ea5fbe5b6a",
        },
        SelfHostingScenario {
            id: "atelier-cheap-first-escalation",
            title: "Cheap-first authoring escalation",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "cheap-first",
                "route-attempts",
                "escalation",
                "route-diagnostic",
                "checked-form",
            ],
            roles: &["contract-author", "validator"],
            cassette_events: &[
                "contract-native:route:cheap-downshift:malformed",
                "contract-native:route:cheap-downshift:diagnostic",
                "contract-native:route:escalation-downshift:accepted",
                "contract-native:route:attempts=2",
            ],
            cassette_hash: "fnv1a64:509ae4bba20e5cfd",
        },
    ]
}

/// Validate offline runner, evidence, and replay-hash invariants.
pub fn validate_self_hosting_scenarios(scenarios: &[SelfHostingScenario]) -> Vec<String> {
    let mut failures = Vec::new();
    for scenario in scenarios {
        if scenario.runner_mode != "fake" && scenario.runner_mode != "cassette" {
            failures.push(format!("{} uses unsupported runner", scenario.id));
        }
        if scenario.live_model {
            failures.push(format!("{} uses a live model", scenario.id));
        }
        if scenario.network {
            failures.push(format!("{} uses network access", scenario.id));
        }
        if scenario.evidence.is_empty() {
            failures.push(format!("{} has no evidence", scenario.id));
        }
        let actual_hash = cassette_content_hash(scenario.cassette_events);
        if actual_hash != scenario.cassette_hash {
            failures.push(format!(
                "{} cassette hash mismatch: expected {}, got {}",
                scenario.id, scenario.cassette_hash, actual_hash
            ));
        }
    }
    failures
}

/// Stable FNV-1a hash used by self-hosting cassette fixtures.
pub fn cassette_content_hash(events: &[&str]) -> String {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(event.as_bytes());
    }
    format!("fnv1a64:{}", fnv1a64_hex(&bytes))
}
