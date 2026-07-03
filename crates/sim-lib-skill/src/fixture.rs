use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};

use crate::{SkillCard, SkillEventSink, SkillTransport};

/// Deterministic behavior a [`FixtureTransport`] runs for an operation.
#[derive(Clone, Debug)]
pub enum FixtureBehavior {
    /// Returns the call arguments unchanged.
    EchoArgs,
    /// Returns the numeric sum of the argument list.
    SumNumbers,
    /// Returns a fixed string regardless of arguments.
    ConstantString(String),
}

/// In-memory skill transport for tests and examples.
///
/// Each registered operation maps to a [`FixtureBehavior`]; calls are counted
/// so tests can assert on dispatch.
pub struct FixtureTransport {
    id: String,
    handlers: Mutex<BTreeMap<String, FixtureBehavior>>,
    calls: AtomicUsize,
}

impl FixtureTransport {
    /// Creates an empty fixture transport with the given `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            handlers: Mutex::new(BTreeMap::new()),
            calls: AtomicUsize::new(0),
        }
    }

    /// Registers `behavior` for `operation`, replacing any existing handler.
    pub fn insert(&self, operation: impl Into<String>, behavior: FixtureBehavior) -> Result<()> {
        self.handlers
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture skill transport"))?
            .insert(operation.into(), behavior);
        Ok(())
    }

    /// Returns the number of calls dispatched through this transport.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SkillTransport for FixtureTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "fixture"
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        Ok(Vec::new())
    }

    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        let behavior = self
            .handlers
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture skill transport"))?
            .get(&card.operation)
            .cloned()
            .ok_or_else(|| {
                Error::Eval(format!(
                    "fixture transport {} has no operation {}",
                    self.id, card.operation
                ))
            })?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        match behavior {
            FixtureBehavior::EchoArgs => Ok(args),
            FixtureBehavior::SumNumbers => sum_number_args(cx, args),
            FixtureBehavior::ConstantString(text) => cx.factory().string(text),
        }
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/health"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
            (
                Symbol::new("calls"),
                cx.factory().string(self.call_count().to_string())?,
            ),
        ])
    }
}

fn sum_number_args(cx: &mut Cx, args: Value) -> Result<Value> {
    let Expr::List(items) = args.object().as_expr(cx)? else {
        return Err(Error::TypeMismatch {
            expected: "argument list",
            found: "non-list",
        });
    };
    let mut sum = 0.0;
    for item in items {
        let Expr::Number(number) = item else {
            return Err(Error::TypeMismatch {
                expected: "number",
                found: "non-number",
            });
        };
        sum += number
            .canonical
            .parse::<f64>()
            .map_err(|err| Error::Eval(format!("invalid number literal: {err}")))?;
    }
    let canonical = if sum.fract() == 0.0 {
        format!("{}", sum as i64)
    } else {
        sum.to_string()
    };
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), canonical)
}
