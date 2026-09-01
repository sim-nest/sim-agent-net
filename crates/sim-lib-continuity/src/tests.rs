use crate::*;
use sim_citizen::{CitizenRegistry, run_registry_conformance_expecting};
use sim_kernel::{Expr, Symbol, testing::bare_cx as cx};
fn plan() -> ContinuityPlan {
    ContinuityPlan {
        schema_version: 1,
        plan_id: Symbol::qualified("continuity.plan", "hostile"),
        roles: vec![RoleDemand {
            role: Symbol::new("root"),
            root: true,
            required_services: vec![Symbol::new("voice")],
            fallbacks: vec![],
        }],
        available_services: vec![Symbol::new("voice")],
        max_freshness: 3,
        retention_turns: 16,
        disclosure: vec![],
        network: NetworkPolicy::Offline,
        allowed_network_routes: vec![],
    }
}
fn event(id: &str, seq: u64, kind: &str) -> ContinuityEvent {
    ContinuityEvent {
        event_id: Symbol::qualified("event", id),
        sequence: seq,
        logical_time: seq,
        kind: Symbol::new(kind),
        role: Symbol::new("root"),
        lease: None,
        payload: Expr::Nil,
        disclosure: None,
    }
}
#[test]
fn citizens_have_shapes_and_general_codec_round_trip() {
    let mut r = CitizenRegistry::new();
    register_citizens(&mut r).unwrap();
    run_registry_conformance_expecting(
        &mut cx(),
        &r,
        &[
            "continuity/Plan",
            "continuity/RoleDemand",
            "continuity/RouteLease",
            "continuity/Turn",
            "continuity/Event",
            "continuity/Intent",
            "continuity/Refusal",
        ],
    )
    .unwrap()
}
#[test]
fn hostile_replay_is_stable_and_effect_free() {
    let p = plan();
    let mut j = MemoryJournal::default();
    let mut s = ContinuityState::default();
    s = j.accept(&p, &s, event("a", 0, "observed")).unwrap();
    assert_eq!(j.accept(&p, &s, event("a", 0, "observed")).unwrap(), s);
    assert!(matches!(
        j.accept(&p, &s, event("future", 3, "observed")),
        Err(JournalError::Refused(_))
    ));
    s = j.accept(&p, &s, event("cancel", 1, "cancel")).unwrap();
    assert!(matches!(
        j.accept(&p, &s, event("late", 2, "observed")),
        Err(JournalError::Refused(_))
    ));
    let rebuilt = rebuild(&p, j.turns()).unwrap();
    assert_eq!(rebuilt, s);
    assert_eq!(
        rebuilt
            .turns
            .iter()
            .flat_map(|t| &t.intents)
            .collect::<Vec<_>>(),
        s.turns.iter().flat_map(|t| &t.intents).collect::<Vec<_>>()
    )
}
#[test]
fn policies_fail_closed() {
    let mut p = plan();
    p.roles.push(p.roles[0].clone());
    assert!(p.validate().is_err());
    assert!(matches!(
        migrate(0, Expr::Nil),
        Err(MigrationError::UnsupportedVersion(0))
    ))
}
