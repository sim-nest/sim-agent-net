use super::*;
use sim_kernel::{Expr, Symbol, testing::eager_cx as cx};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct SplitSeat {
    id: &'static str,
    delay_ms: u64,
    plan_order: Arc<Mutex<Vec<&'static str>>>,
    completion_order: Arc<Mutex<Vec<&'static str>>>,
    calls: Arc<AtomicUsize>,
    failure: Option<&'static str>,
}

impl FanoutSeat for SplitSeat {
    fn seat_id(&self) -> &str {
        self.id
    }

    fn max_in_flight(&self) -> Option<u32> {
        Some(1)
    }

    fn plan(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<PlannedSeat> {
        self.plan_order.lock().unwrap().push(self.id);
        let id = self.id;
        let delay = self.delay_ms;
        let completion_order = self.completion_order.clone();
        let calls = self.calls.clone();
        let failure = self.failure;
        Ok(PlannedSeat::parallel(
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(delay));
                completion_order.lock().unwrap().push(id);
                failure.map_or(Ok(id), |message| Err(Error::Eval(message.to_owned())))
            },
            |_cx, id| {
                Ok(ModelResponse::new(
                    Symbol::new(id),
                    "fixture",
                    vec![Expr::String(id.to_owned())],
                    Symbol::new("stop"),
                ))
            },
        ))
    }
}

#[test]
fn parallel_dispatch_is_bounded_by_slowest_seat_and_lands_in_request_order() {
    let plans = Arc::new(Mutex::new(Vec::new()));
    let completions = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let slow = SplitSeat {
        id: "slow",
        delay_ms: 180,
        plan_order: plans.clone(),
        completion_order: completions.clone(),
        calls: calls.clone(),
        failure: None,
    };
    let fast = SplitSeat {
        id: "fast",
        delay_ms: 100,
        plan_order: plans.clone(),
        completion_order: completions.clone(),
        calls,
        failure: None,
    };
    let pool = WorkerPool::new(2);
    let started = Instant::now();
    let report = Fanout::default().execute_on(
        &mut cx(),
        &[&slow, &fast],
        FanoutMode::All,
        ThreadMode::Pool,
        ModelRequest::default(),
        &pool,
    );

    assert!(started.elapsed() < Duration::from_millis(240));
    assert_eq!(*plans.lock().unwrap(), vec!["slow", "fast"]);
    assert_eq!(*completions.lock().unwrap(), vec!["fast", "slow"]);
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| row.seat.as_str())
            .collect::<Vec<_>>(),
        vec!["slow", "fast"]
    );
    assert!(report.rows.iter().all(|row| row.result.is_ok()));
}

#[test]
fn quota_cooldown_uses_injected_time_without_sleeping() {
    let clock = Arc::new(ManualFanoutClock::new(10));
    let fanout = Fanout::new(clock.clone(), Duration::from_millis(50));
    let seat = SplitSeat {
        id: "limited",
        delay_ms: 0,
        plan_order: Arc::new(Mutex::new(Vec::new())),
        completion_order: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(AtomicUsize::new(0)),
        failure: Some("quota exceeded"),
    };
    let pool = WorkerPool::new(1);
    let first = fanout.execute_on(
        &mut cx(),
        &[&seat],
        FanoutMode::All,
        ThreadMode::Pool,
        ModelRequest::default(),
        &pool,
    );
    assert!(first.rows[0].result.is_err());
    let cooling = fanout.execute_on(
        &mut cx(),
        &[&seat],
        FanoutMode::All,
        ThreadMode::Pool,
        ModelRequest::default(),
        &pool,
    );
    assert_eq!(cooling.rows[0].status, FanoutStatus::Unavailable);
    clock.advance(50);
    let retried = fanout.execute_on(
        &mut cx(),
        &[&seat],
        FanoutMode::All,
        ThreadMode::Pool,
        ModelRequest::default(),
        &pool,
    );
    assert_eq!(retried.rows[0].status, FanoutStatus::Parallel);
}

#[test]
fn unsplittable_seat_is_reported_as_serialized() {
    struct LocalSeat;
    impl FanoutSeat for LocalSeat {
        fn seat_id(&self) -> &str {
            "local"
        }
        fn plan(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<PlannedSeat> {
            Ok(PlannedSeat::serialized(|_cx| {
                Ok(ModelResponse::new(
                    Symbol::new("local"),
                    "fixture",
                    Vec::new(),
                    Symbol::new("stop"),
                ))
            }))
        }
    }
    let report = Fanout::default().execute(
        &mut cx(),
        &[&LocalSeat],
        FanoutMode::FirstGood,
        ThreadMode::Pool,
        ModelRequest::default(),
    );
    assert_eq!(report.rows[0].status, FanoutStatus::Serialized);
    assert_eq!(report.selected, Some(0));
}
