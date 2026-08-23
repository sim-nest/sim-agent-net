//! Provider-neutral executable conformance for opened seats.

use crate::ProviderSeatCard;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelRequest, ModelRunner};

/// A provider behavior which must be declared before it may be requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderCapability {
    /// Incremental model events.
    Streaming,
    /// Tool declarations and tool-call results.
    Tools,
    /// Caller-declared structured output.
    StructuredOutput,
    /// Token or cost accounting.
    Usage,
    /// Effects within an explicitly bounded workspace.
    WorkspaceEffects,
}

impl ProviderCapability {
    fn symbol(self) -> Symbol {
        Symbol::new(match self {
            Self::Streaming => "streaming",
            Self::Tools => "tools",
            Self::StructuredOutput => "structured-output",
            Self::Usage => "usage",
            Self::WorkspaceEffects => "workspace-effects",
        })
    }
}

/// Open-card key containing the capability symbols advertised by one seat.
pub fn provider_capabilities_key() -> Expr {
    Expr::Symbol(Symbol::qualified("provider", "capabilities"))
}

/// Evidence returned after the common provider checks pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConformanceReport {
    /// Stable seat identity which was checked.
    pub seat: String,
    /// Stable runner identity observed in the successful response.
    pub runner: Symbol,
    /// Number of streaming events observed, including the final event.
    pub stream_events: usize,
}

/// The one conformance harness shared by direct API, CLI broker, local service,
/// and third-party provider adapters.
pub struct ProviderConformanceHarness<'a> {
    seat: &'a ProviderSeatCard,
    runner: &'a dyn ModelRunner,
}

impl<'a> ProviderConformanceHarness<'a> {
    /// Binds an already opened runner to the exact seat card which opened it.
    pub fn new(seat: &'a ProviderSeatCard, runner: &'a dyn ModelRunner) -> Self {
        Self { seat, runner }
    }

    /// Refuses an unsupported behavior before invoking the runner.
    pub fn require(&self, capability: ProviderCapability) -> Result<()> {
        if self.capabilities().contains(&capability.symbol()) {
            Ok(())
        } else {
            Err(Error::Eval(format!(
                "provider seat {} does not support {}",
                self.seat.seat,
                capability.symbol()
            )))
        }
    }

    /// Runs the common success, structured-output, usage, streaming, error, and
    /// cancellation assertions without inspecting a vendor or family enum.
    pub fn run(&self, cx: &mut Cx) -> Result<ProviderConformanceReport> {
        for capability in [
            ProviderCapability::Streaming,
            ProviderCapability::Tools,
            ProviderCapability::StructuredOutput,
            ProviderCapability::Usage,
            ProviderCapability::WorkspaceEffects,
        ] {
            self.require(capability)?;
        }

        let checked = request("checked");
        let response = self.runner.infer(cx, checked.clone())?;
        if response.content != [Expr::String("checked-answer".to_owned())] {
            return Err(Error::Eval(format!(
                "provider seat {} violated the checked output Shape",
                self.seat.seat
            )));
        }
        if response.usage.is_none() {
            return Err(Error::Eval(format!(
                "provider seat {} omitted declared usage",
                self.seat.seat
            )));
        }
        for evidence in ["tool-call", "workspace-effect"] {
            if !response.extra.iter().any(|(key, value)| {
                key == &Expr::Symbol(Symbol::qualified("provider", evidence))
                    && value == &Expr::Bool(true)
            }) {
                return Err(Error::Eval(format!(
                    "provider seat {} omitted declared {evidence} evidence",
                    self.seat.seat
                )));
            }
        }

        let mut events = CountingSink::default();
        self.runner.infer_stream(cx, checked, &mut events)?;
        if events.count == 0 {
            return Err(Error::Eval(
                "declared streaming emitted no events".to_owned(),
            ));
        }
        assert_refusal(self.runner.infer(cx, request("error")), "fixture-error")?;
        assert_refusal(
            self.runner.infer(cx, request("cancel")),
            "cancelled-before-effect",
        )?;

        Ok(ProviderConformanceReport {
            seat: self.seat.seat.to_string(),
            runner: response.runner,
            stream_events: events.count,
        })
    }

    fn capabilities(&self) -> Vec<Symbol> {
        self.seat
            .extra
            .iter()
            .find_map(|(key, value)| (key == &provider_capabilities_key()).then_some(value))
            .and_then(|value| match value {
                Expr::List(items) => Some(
                    items
                        .iter()
                        .filter_map(|item| match item {
                            Expr::Symbol(symbol) => Some(symbol.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }
}

fn request(case: &str) -> ModelRequest {
    let mut request =
        ModelRequest::new(Expr::String("checked provider request".to_owned()), vec![]);
    request.extra.push((
        Expr::Symbol(Symbol::qualified("provider", "conformance-case")),
        Expr::Symbol(Symbol::new(case)),
    ));
    for requirement in ["tool", "output-shape", "workspace"] {
        request.extra.push((
            Expr::Symbol(Symbol::qualified("provider", requirement)),
            Expr::Bool(true),
        ));
    }
    request
}

fn assert_refusal<T>(result: Result<T>, expected: &str) -> Result<()> {
    match result {
        Err(error) if error.to_string().contains(expected) => Ok(()),
        Err(error) => Err(Error::Eval(format!("wrong provider refusal: {error}"))),
        Ok(_) => Err(Error::Eval(format!(
            "provider accepted required refusal case {expected}"
        ))),
    }
}

#[derive(Default)]
struct CountingSink {
    count: usize,
}

impl ModelEventSink for CountingSink {
    fn emit(&mut self, _event: ModelEvent) -> Result<()> {
        self.count += 1;
        Ok(())
    }
}
