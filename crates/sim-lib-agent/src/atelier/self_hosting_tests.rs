use super::self_hosting::{
    cassette_content_hash, self_hosting_scenarios, validate_self_hosting_scenarios,
};

#[test]
fn self_hosting_scenarios_are_offline_and_hash_checked() {
    let scenarios = self_hosting_scenarios();
    assert_eq!(scenarios.len(), 5);
    assert!(validate_self_hosting_scenarios(&scenarios).is_empty());
    assert!(scenarios.iter().all(|scenario| !scenario.live_model));
    assert!(scenarios.iter().all(|scenario| !scenario.network));
}

#[test]
fn self_hosting_scenarios_cover_atelier_surfaces() {
    let scenarios = self_hosting_scenarios();
    let evidence = scenarios
        .iter()
        .flat_map(|scenario| scenario.evidence.iter().copied())
        .collect::<Vec<_>>();
    for required in [
        "radar",
        "codec-prism",
        "guideline-firewall",
        "validation",
        "docs",
        "pin-plan",
        "human-gate",
        "replay-hash",
    ] {
        assert!(evidence.contains(&required), "missing {required}");
    }
}

#[test]
fn change_capsule_scenario_lists_every_review_role() {
    let scenarios = self_hosting_scenarios();
    let capsule = scenarios
        .iter()
        .find(|scenario| scenario.id == "atelier-change-capsule")
        .expect("change capsule scenario");
    for role in [
        "cartographer",
        "editor",
        "guard",
        "validator",
        "docs-agent",
        "pin-agent",
        "reviewer",
        "human-gate",
    ] {
        assert!(capsule.roles.contains(&role), "missing {role}");
    }
    assert_eq!(
        cassette_content_hash(capsule.cassette_events),
        capsule.cassette_hash
    );
}
