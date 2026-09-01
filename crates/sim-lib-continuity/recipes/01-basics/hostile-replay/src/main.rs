use sim_kernel::{Expr, Symbol};
use sim_lib_continuity::*;

fn event(id: &str, sequence: u64, kind: &str) -> ContinuityEvent {
    ContinuityEvent {
        event_id: Symbol::qualified("event", id),
        sequence,
        logical_time: sequence,
        kind: Symbol::new(kind),
        role: Symbol::new("root"),
        lease: None,
        payload: Expr::Nil,
        disclosure: None,
    }
}

fn main() {
    let plan = ContinuityPlan {
        retention_turns: 2,
        ..ContinuityPlan::default()
    };
    let mut journal = MemoryJournal::default();
    let mut state = ContinuityState::default();
    state = journal
        .accept(&plan, &state, event("first", 0, "observed"))
        .unwrap();
    assert_eq!(
        journal
            .accept(&plan, &state, event("first", 0, "observed"))
            .unwrap(),
        state
    );
    assert!(
        journal
            .accept(&plan, &state, event("reordered", 2, "observed"))
            .is_err()
    );
    state = journal
        .accept(&plan, &state, event("cancel", 1, "cancel"))
        .unwrap();
    assert!(
        journal
            .accept(&plan, &state, event("late", 2, "observed"))
            .is_err()
    );
    let rebuilt = rebuild(&plan, journal.turns()).unwrap();
    assert_eq!(rebuilt, state);
    let intents = rebuilt
        .turns
        .iter()
        .map(|turn| turn.intents.len())
        .sum::<usize>();
    println!(
        "stable turns: {}; stable intents: {intents}",
        rebuilt.turns.len()
    );
}
