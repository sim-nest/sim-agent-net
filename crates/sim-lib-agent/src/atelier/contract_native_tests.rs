use sim_kernel::Symbol;

use super::{
    AgentMission, AtelierBackend, CONTRACT_NATIVE_SCHEMA, ContractNativeAtelierReport,
    deterministic_contract_native_report,
};

#[test]
fn source_radar_is_the_default_backend() {
    assert_eq!(AtelierBackend::default(), AtelierBackend::SourceRadar);
    assert_eq!(
        AtelierBackend::parse("source-radar"),
        Some(AtelierBackend::SourceRadar)
    );
    assert_eq!(
        AtelierBackend::parse("contract-native"),
        Some(AtelierBackend::ContractNative)
    );
    assert_eq!(AtelierBackend::parse("source"), None);
}

#[test]
fn deterministic_contract_native_report_caches_backend_evidence() {
    let report = deterministic_contract_native_report();
    assert_eq!(report.backend, AtelierBackend::ContractNative);
    assert_eq!(report.deck.cards, 4);
    assert_eq!(report.projection.tokens, 114);
    assert_eq!(report.grammar.dialect, "shapegrammar");
    assert!(report.grammar.strict);
    assert_eq!(report.route_attempts.len(), 2);
    assert!(report.cassette_hash.starts_with("fnv1a64:"));

    let json = report.to_json();
    assert_eq!(json["schema"], CONTRACT_NATIVE_SCHEMA);
    assert_eq!(json["backend"], "contract-native");
    assert_eq!(json["projection"]["summary_only"], 1);
    assert_eq!(json["grammar"]["target_codec"], "codec:lisp");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("source-free"))
    );
}

#[test]
fn contract_native_report_keeps_guard_denials() {
    let report = deterministic_contract_native_report();
    let ids = denial_ids(&report);
    assert!(ids.contains(&"meta-workspace-edit".to_owned()));
    assert!(ids.contains(&"cross-repo-write".to_owned()));
    assert!(ids.contains(&"github-outward-action".to_owned()));
    assert!(
        report
            .guard_denials
            .iter()
            .any(|denial| denial.reason.contains(".meta-workspace"))
    );
    assert!(
        report
            .guard_denials
            .iter()
            .any(|denial| denial.reason.contains("mission lease"))
    );
    assert!(
        report
            .guard_denials
            .iter()
            .any(|denial| denial.reason.contains("GitHub remote"))
    );
}

#[test]
fn guard_denials_are_source_free_mission_checks() {
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "contract-native-test"),
        "sim-agent-net",
    );
    let denials = super::contract_native_guard_denials(&mission);
    assert_eq!(denials.len(), 3);
    assert!(denials.iter().all(|denial| !denial.action.is_empty()));
    assert!(denials.iter().all(|denial| !denial.reason.is_empty()));
}

fn denial_ids(report: &ContractNativeAtelierReport) -> Vec<String> {
    report
        .guard_denials
        .iter()
        .map(|denial| denial.id.clone())
        .collect()
}
