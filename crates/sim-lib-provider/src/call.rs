use sim_kernel::{Cx, Result};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};

/// A fully planned provider call whose payload can leave the runtime thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCall<Call> {
    /// Provider-specific dispatch payload.
    pub payload: Call,
}

impl<Call> ProviderCall<Call> {
    /// Wraps a provider-specific dispatch payload.
    pub fn new(payload: Call) -> Self {
        Self { payload }
    }
}

/// A completed provider dispatch waiting to be landed on a runtime thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderOutcome<Outcome> {
    /// Provider-specific transport outcome.
    pub payload: Outcome,
}

impl<Outcome> ProviderOutcome<Outcome> {
    /// Wraps a provider-specific transport outcome.
    pub fn new(payload: Outcome) -> Self {
        Self { payload }
    }
}

/// Thread-independent middle stage of provider execution.
pub trait ProviderDispatch {
    /// Fully owned payload produced by planning.
    type Call: Send + 'static;
    /// Fully owned payload consumed by landing.
    type Outcome: Send + 'static;

    /// Performs provider I/O without access to a runtime context.
    fn dispatch(call: ProviderCall<Self::Call>) -> Result<ProviderOutcome<Self::Outcome>>;
}

/// Splits provider execution around the thread-bound runtime context.
pub trait ProviderSeatExecution: ProviderDispatch {
    /// Plans a request on the runtime thread.
    fn plan(&self, cx: &mut Cx, request: ModelRequest) -> Result<ProviderCall<Self::Call>>;

    /// Lands a completed transport outcome on the runtime thread.
    fn land(&self, cx: &mut Cx, outcome: ProviderOutcome<Self::Outcome>) -> Result<ModelResponse>;

    /// Executes the canonical plan, dispatch, and land sequence.
    fn execute(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let call = self.plan(cx, request)?;
        let outcome = Self::dispatch(call)?;
        self.land(cx, outcome)
    }
}

fn _assert_split_records_are_send_static<Call, Outcome>()
where
    Call: Send + 'static,
    Outcome: Send + 'static,
{
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<ProviderCall<Call>>();
    assert_send_static::<ProviderOutcome<Outcome>>();
}
