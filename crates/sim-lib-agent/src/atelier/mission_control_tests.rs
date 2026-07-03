use sim_kernel::{Expr, Symbol};

use super::fake_mission_control_fixture;

#[test]
fn mission_control_fixture_records_replayable_evidence() {
    let fixture = fake_mission_control_fixture().unwrap();

    assert_eq!(
        fixture.mission.id(),
        &Symbol::qualified("agent/mission", "mission-control-fixture")
    );
    assert_eq!(fixture.cassette.cassette().envelopes().len(), 5);
    assert_eq!(
        fixture.cassette.content_hash(),
        fixture.cassette.replay_content_hash().unwrap()
    );
}

#[test]
fn mission_control_fixture_carries_exact_conflict_targets() {
    let fixture = fake_mission_control_fixture().unwrap();

    assert_eq!(fixture.conflicts.len(), 1);
    assert_eq!(fixture.conflicts[0].left.target, "sim-lib-agent");
    assert_eq!(fixture.conflicts[0].right.target, "sim-lib-agent");
    assert!(snapshot_list(&fixture.snapshot, "roles").len() >= 9);
    assert_eq!(snapshot_list(&fixture.snapshot, "conflicts").len(), 1);
}

fn snapshot_list<'a>(expr: &'a Expr, key: &str) -> &'a [Expr] {
    let Expr::Map(entries) = expr else {
        panic!("fixture snapshot must be a map");
    };
    let Some((_, Expr::List(items))) = entries.iter().find(|(entry_key, _)| {
        matches!(entry_key, Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == key)
    }) else {
        panic!("missing list field {key}");
    };
    items
}
