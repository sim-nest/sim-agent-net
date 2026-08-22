use std::{collections::VecDeque, sync::Mutex};

use sim_kernel::{Error, Result};

use super::{DeterministicWallClock, WallClock, WallTimestamp};
use crate::trigger::TriggerState;

// conformance: server wall observations are injectable, bounded, and non-monotonic.

struct ReversingWallClock {
    observations: Mutex<VecDeque<WallTimestamp>>,
}

impl WallClock for ReversingWallClock {
    fn now(&self) -> Result<WallTimestamp> {
        self.observations
            .lock()
            .map_err(|_| Error::PoisonedLock("reversing wall clock"))?
            .pop_front()
            .ok_or_else(|| Error::Eval("reversing wall clock exhausted".to_owned()))
    }
}

#[test]
fn deterministic_wall_clock_is_object_safe_and_never_reads_ambient_time() {
    let clock: Box<dyn WallClock> = Box::new(DeterministicWallClock::new(1_000, 25));
    assert_eq!(clock.now().unwrap().unix_millis(), 1_000);
    assert_eq!(clock.now_ms().unwrap(), 1_025);
}

#[test]
fn deterministic_wall_clock_rejects_overflow_without_wrapping() {
    let clock = DeterministicWallClock::new(u64::MAX, 1);
    let error = clock.now().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("deterministic wall clock overflow")
    );
}

#[test]
fn wall_clock_contract_and_cron_deduplication_accept_backward_observations() {
    let clock: Box<dyn WallClock> = Box::new(ReversingWallClock {
        observations: Mutex::new(VecDeque::from([
            WallTimestamp::from_unix_millis(120_000),
            WallTimestamp::from_unix_millis(60_000),
        ])),
    });
    let later = clock.now().unwrap();
    let earlier = clock.now().unwrap();
    assert!(earlier < later, "wall observations are not a logical clock");

    let mut state = TriggerState::default();
    assert!(state.advance_cron_high_watermark(2));
    assert!(
        !state.advance_cron_high_watermark(1),
        "clock rollback must not fire an already passed cron minute"
    );
    assert!(state.advance_cron_high_watermark(3));
}
