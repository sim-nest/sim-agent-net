//! Deterministic, capacity-aware provider fan-out.

use crate::ProviderSeatExecution;
use sim_kernel::{Cx, Error, Result};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};
use sim_lib_server::{ThreadMode, WorkerPool, default_worker_pool};
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Payload = Box<dyn Any + Send>;
type Dispatch = Box<dyn FnOnce() -> Result<Payload> + Send>;
type Land = Box<dyn FnOnce(&mut Cx, Payload) -> Result<ModelResponse>>;
type Serialized = Box<dyn FnOnce(&mut Cx) -> Result<ModelResponse>>;

/// Completion policy for one fan-out request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FanoutMode {
    /// Return every requested seat row.
    All,
    /// Select the first successful dispatch while retaining deterministic rows.
    FirstGood,
    /// Require this many successful seats.
    Quorum(usize),
}

/// Injectable monotonic time used by seat cooldowns.
pub trait FanoutClock: Send + Sync {
    /// Current logical time in milliseconds.
    fn now_millis(&self) -> u64;
}

/// Wall-clock implementation used outside tests.
#[derive(Debug, Default)]
pub struct SystemFanoutClock;

impl FanoutClock for SystemFanoutClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

/// Manually advanced clock for deterministic tests and modeled execution.
#[derive(Debug, Default)]
pub struct ManualFanoutClock(std::sync::atomic::AtomicU64);

impl ManualFanoutClock {
    /// Creates a clock at `millis`.
    pub fn new(millis: u64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(millis))
    }

    /// Advances the clock without sleeping.
    pub fn advance(&self, millis: u64) {
        self.0
            .fetch_add(millis, std::sync::atomic::Ordering::Relaxed);
    }
}

impl FanoutClock for ManualFanoutClock {
    fn now_millis(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A request planned on the runtime thread.
pub struct PlannedSeat(PlannedSeatKind);

enum PlannedSeatKind {
    Parallel { dispatch: Dispatch, land: Land },
    Serialized(Serialized),
}

impl PlannedSeat {
    /// Plans an existing split provider execution and erases only its owned
    /// dispatch payload types for heterogeneous fan-out.
    pub fn from_execution<S>(seat: Arc<S>, cx: &mut Cx, request: ModelRequest) -> Result<Self>
    where
        S: ProviderSeatExecution + Send + Sync + 'static,
    {
        let call = seat.plan(cx, request)?;
        Ok(Self::parallel(
            move || S::dispatch(call),
            move |cx, outcome| seat.land(cx, outcome),
        ))
    }

    /// Builds a split dispatch/land plan.
    pub fn parallel<D, O, L>(dispatch: D, land: L) -> Self
    where
        D: FnOnce() -> Result<O> + Send + 'static,
        O: Any + Send + 'static,
        L: FnOnce(&mut Cx, O) -> Result<ModelResponse> + 'static,
    {
        Self(PlannedSeatKind::Parallel {
            dispatch: Box::new(move || dispatch().map(|value| Box::new(value) as Payload)),
            land: Box::new(move |cx, payload| {
                let value = payload.downcast::<O>().map_err(|_| {
                    Error::Eval("provider fan-out outcome type mismatch".to_owned())
                })?;
                land(cx, *value)
            }),
        })
    }

    /// Builds a plan for a seat that cannot leave the runtime thread.
    pub fn serialized<F>(execute: F) -> Self
    where
        F: FnOnce(&mut Cx) -> Result<ModelResponse> + 'static,
    {
        Self(PlannedSeatKind::Serialized(Box::new(execute)))
    }
}

/// One independently addressable provider execution seat.
pub trait FanoutSeat {
    /// Stable seat label used in reports and lease accounting.
    fn seat_id(&self) -> &str;

    /// Maximum concurrent requests for this seat. `None` means unbounded.
    fn max_in_flight(&self) -> Option<u32> {
        None
    }

    /// Plans all runtime-context work before any parallel dispatch begins.
    fn plan(&self, cx: &mut Cx, request: ModelRequest) -> Result<PlannedSeat>;
}

/// How one requested seat was executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FanoutStatus {
    /// The seat used an off-thread dispatch.
    Parallel,
    /// The seat could not leave the runtime thread.
    Serialized,
    /// The seat was rejected by its in-flight or cooldown lease.
    Unavailable,
}

/// Deterministic result row for one requested seat.
#[derive(Debug)]
pub struct FanoutRow {
    /// Seat label supplied by the seat.
    pub seat: String,
    /// Execution disposition.
    pub status: FanoutStatus,
    /// Response or seat-specific failure.
    pub result: Result<ModelResponse>,
}

/// Ordered result of one fan-out operation.
#[derive(Debug)]
pub struct FanoutReport {
    /// Rows in the exact order seats were requested.
    pub rows: Vec<FanoutRow>,
    /// Index of the selected successful row, if the mode selected one.
    pub selected: Option<usize>,
}

#[derive(Default)]
struct LeaseState {
    in_flight: u32,
    cooldown_until: u64,
}

struct PlannedRow {
    seat: String,
    status: FanoutStatus,
    plan: Option<PlannedSeat>,
    error: Option<Error>,
}

/// Shared provider fan-out engine with per-seat lease state.
pub struct Fanout {
    clock: Arc<dyn FanoutClock>,
    cooldown: Duration,
    leases: Mutex<HashMap<String, LeaseState>>,
}

impl Default for Fanout {
    fn default() -> Self {
        Self::new(Arc::new(SystemFanoutClock), Duration::from_secs(30))
    }
}

impl Fanout {
    /// Creates an engine with injectable time and rate-limit cooldown.
    pub fn new(clock: Arc<dyn FanoutClock>, cooldown: Duration) -> Self {
        Self {
            clock,
            cooldown,
            leases: Mutex::new(HashMap::new()),
        }
    }

    /// Plans every seat, dispatches eligible work concurrently, then lands and
    /// reports every row in requested seat order.
    pub fn execute(
        &self,
        cx: &mut Cx,
        seats: &[&dyn FanoutSeat],
        mode: FanoutMode,
        thread: ThreadMode,
        request: ModelRequest,
    ) -> FanoutReport {
        self.execute_on(cx, seats, mode, thread, request, default_worker_pool())
    }

    /// Executes using an explicit pool, primarily for bounded hosts and tests.
    pub fn execute_on(
        &self,
        cx: &mut Cx,
        seats: &[&dyn FanoutSeat],
        mode: FanoutMode,
        thread: ThreadMode,
        request: ModelRequest,
        pool: &WorkerPool,
    ) -> FanoutReport {
        let parallel = supports_parallel(&thread);
        let mut planned = Vec::with_capacity(seats.len());
        for seat in seats {
            let id = seat.seat_id().to_owned();
            if let Err(error) = self.acquire(&id, seat.max_in_flight()) {
                planned.push(PlannedRow {
                    seat: id,
                    status: FanoutStatus::Unavailable,
                    plan: None,
                    error: Some(error),
                });
                continue;
            }
            match seat.plan(cx, request.clone()) {
                Ok(PlannedSeat(PlannedSeatKind::Parallel { dispatch, land })) if parallel => {
                    planned.push(PlannedRow {
                        seat: id,
                        status: FanoutStatus::Parallel,
                        plan: Some(PlannedSeat(PlannedSeatKind::Parallel { dispatch, land })),
                        error: None,
                    })
                }
                Ok(PlannedSeat(PlannedSeatKind::Parallel { dispatch, land })) => {
                    planned.push(PlannedRow {
                        seat: id,
                        status: FanoutStatus::Serialized,
                        plan: Some(PlannedSeat(PlannedSeatKind::Parallel { dispatch, land })),
                        error: None,
                    })
                }
                Ok(plan @ PlannedSeat(PlannedSeatKind::Serialized(_))) => {
                    planned.push(PlannedRow {
                        seat: id,
                        status: FanoutStatus::Serialized,
                        plan: Some(plan),
                        error: None,
                    })
                }
                Err(error) => {
                    self.release_planning(&id);
                    planned.push(PlannedRow {
                        seat: id,
                        status: FanoutStatus::Unavailable,
                        plan: None,
                        error: Some(error),
                    });
                }
            }
        }

        let (tx, rx) = mpsc::channel();
        let mut outcomes: HashMap<usize, Result<Payload>> = HashMap::new();
        let mut lands: HashMap<usize, Land> = HashMap::new();
        let mut parallel_count = 0;
        for (index, row) in planned.iter_mut().enumerate() {
            let Some(PlannedSeat(PlannedSeatKind::Parallel { dispatch, land })) = row.plan.take()
            else {
                continue;
            };
            lands.insert(index, land);
            if row.status == FanoutStatus::Parallel {
                parallel_count += 1;
                let tx = tx.clone();
                pool.execute(move || {
                    let _ = tx.send((index, dispatch()));
                });
            } else {
                outcomes.insert(index, dispatch());
            }
        }
        drop(tx);
        for _ in 0..parallel_count {
            if let Ok((index, outcome)) = rx.recv() {
                outcomes.insert(index, outcome);
            }
        }

        let mut rows = Vec::with_capacity(planned.len());
        for (index, mut row) in planned.into_iter().enumerate() {
            let result = if let Some(error) = row.error.take() {
                Err(error)
            } else if let Some(PlannedSeat(PlannedSeatKind::Serialized(execute))) = row.plan.take()
            {
                execute(cx)
            } else {
                match outcomes.remove(&index) {
                    Some(Ok(outcome)) => lands
                        .remove(&index)
                        .expect("landing stage accompanies dispatch")(
                        cx, outcome
                    ),
                    Some(Err(error)) => Err(error),
                    None => Err(Error::Eval(
                        "provider fan-out lost dispatch outcome".to_owned(),
                    )),
                }
            };
            if row.status != FanoutStatus::Unavailable {
                self.release(&row.seat, &result);
            }
            rows.push(FanoutRow {
                seat: row.seat,
                status: row.status,
                result,
            });
        }
        let required = match mode {
            FanoutMode::All => rows.len(),
            FanoutMode::FirstGood => 1,
            FanoutMode::Quorum(required) => required,
        };
        let successes: Vec<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.result.is_ok().then_some(index))
            .collect();
        let selected = (successes.len() >= required)
            .then(|| successes.first().copied())
            .flatten();
        FanoutReport { rows, selected }
    }

    fn acquire(&self, seat: &str, limit: Option<u32>) -> Result<()> {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let lease = leases.entry(seat.to_owned()).or_default();
        if self.clock.now_millis() < lease.cooldown_until {
            return Err(Error::Eval(format!("provider seat {seat} is cooling down")));
        }
        if limit.is_some_and(|limit| lease.in_flight >= limit) {
            return Err(Error::Eval(format!(
                "provider seat {seat} is at max in-flight"
            )));
        }
        lease.in_flight += 1;
        Ok(())
    }

    fn release(&self, seat: &str, result: &Result<ModelResponse>) {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let lease = leases.entry(seat.to_owned()).or_default();
        lease.in_flight = lease.in_flight.saturating_sub(1);
        if result.as_ref().err().is_some_and(|error| {
            let message = error.to_string().to_ascii_lowercase();
            message.contains("rate limit") || message.contains("quota")
        }) {
            let cooldown = self.cooldown.as_millis().try_into().unwrap_or(u64::MAX);
            lease.cooldown_until = self.clock.now_millis().saturating_add(cooldown);
        }
    }

    fn release_planning(&self, seat: &str) {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let lease = leases.entry(seat.to_owned()).or_default();
        lease.in_flight = lease.in_flight.saturating_sub(1);
    }
}

fn supports_parallel(mode: &ThreadMode) -> bool {
    match mode {
        ThreadMode::Spawn | ThreadMode::Pool => true,
        ThreadMode::Coroutine(inner) => supports_parallel(inner),
        ThreadMode::Main | ThreadMode::Coop => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{DefaultFactory, EagerPolicy, Expr, Symbol};
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

    fn cx() -> Cx {
        Cx::new(
            Arc::new(EagerPolicy),
            Arc::new(DefaultFactory),
            sim_kernel::HandleSeed::new(0x4641_4e4f),
        )
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
}
