use std::sync::Arc;

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Object, ObjectCompat, Result,
    ShapeId, ShapeRef, Symbol, Value,
};
use sim_shape::check_value_report;

#[cfg(any(feature = "cache", feature = "cassette"))]
use crate::record::{
    SkillAuditEntry, SkillCallState, cache_read_allowed, cache_write_allowed, call_key,
    cassette_record_allowed, cassette_replay_allowed, value_from_recorded_expr,
};
use crate::{SkillCard, SkillEventSink, SkillTransport};

/// Callable runtime object that invokes a single bound skill.
///
/// A `SkillCallable` pairs a [`SkillCard`] with the [`SkillTransport`] that
/// runs it. Calling it checks the requested capabilities, validates arguments
/// against the card's input shape, dispatches through the transport (honoring
/// any cache/cassette policy when those features are enabled), and validates
/// the result against the output shape. It is registered as an ordinary
/// callable function value, so agents call a skill exactly like any other
/// function.
#[derive(Clone)]
pub struct SkillCallable {
    card: SkillCard,
    transport: Arc<dyn SkillTransport>,
    #[cfg(any(feature = "cache", feature = "cassette"))]
    state: SkillCallState,
    #[cfg(any(feature = "cache", feature = "cassette"))]
    registry: Option<crate::SkillRegistry>,
}

impl SkillCallable {
    /// Creates a callable for `card` that dispatches through `transport`.
    pub fn new(card: SkillCard, transport: Arc<dyn SkillTransport>) -> Self {
        Self {
            card,
            transport,
            #[cfg(any(feature = "cache", feature = "cassette"))]
            state: SkillCallState::default(),
            #[cfg(any(feature = "cache", feature = "cassette"))]
            registry: None,
        }
    }

    #[cfg(any(feature = "cache", feature = "cassette"))]
    pub(crate) fn new_bound(
        card: SkillCard,
        transport: Arc<dyn SkillTransport>,
        registry: crate::SkillRegistry,
    ) -> Self {
        Self {
            card,
            transport,
            state: SkillCallState::default(),
            registry: Some(registry),
        }
    }

    /// Returns the [`SkillCard`] this callable invokes.
    pub fn card(&self) -> &SkillCard {
        &self.card
    }

    /// Invokes the skill with already-evaluated `args`.
    ///
    /// Requires the skill's capabilities, checks the arguments against the
    /// card's input shape, dispatches through the transport (applying cache
    /// and cassette policy when those features are enabled), and checks the
    /// result against the output shape. The optional `events` sink receives
    /// any streaming events the transport emits during the call.
    pub fn call_values(
        &self,
        cx: &mut Cx,
        args: Vec<Value>,
        events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        cx.require(&crate::skill_call_capability())?;
        for capability in &self.card.capabilities {
            cx.require(capability)?;
        }
        let args_value = self.check_args(cx, &args)?;
        #[cfg(any(feature = "cache", feature = "cassette"))]
        {
            self.call_recorded(cx, args_value, events)
        }
        #[cfg(not(any(feature = "cache", feature = "cassette")))]
        {
            let value = self.transport.call(cx, &self.card, args_value, events)?;
            self.check_result(cx, value)
        }
    }

    #[cfg(any(feature = "cache", feature = "cassette"))]
    fn call_recorded(
        &self,
        cx: &mut Cx,
        args_value: Value,
        events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        let key = call_key(cx, &self.card, &args_value)?;
        let cache_enabled = self.card.policy.idempotent;
        if cache_enabled
            && cache_read_allowed(&self.card.policy.cache)
            && let Some(expr) = self.state.cache_lookup(&key)?
        {
            self.record_audit("cache-hit", "hit", "disabled")?;
            let value = value_from_recorded_expr(cx, expr)?;
            return self.check_result(cx, value);
        }
        if cassette_replay_allowed(&self.card.policy.cassette)
            && let Some(expr) = self.state.cassette_lookup(&key)?
        {
            self.record_audit("cassette-hit", "disabled", "hit")?;
            let value = value_from_recorded_expr(cx, expr)?;
            return self.check_result(cx, value);
        }
        if matches!(
            self.card.policy.cassette,
            crate::SkillCassetteMode::ReplayOnly
        ) {
            self.record_audit("cassette-miss", "disabled", "miss")?;
            return Err(Error::Eval(format!(
                "skill cassette replay miss for {}",
                self.card.id
            )));
        }
        let value = match self.transport.call(cx, &self.card, args_value, events) {
            Ok(value) => value,
            Err(error) => {
                self.record_audit("error", "miss", "miss")?;
                return Err(error);
            }
        };
        let checked = match self.check_result(cx, value) {
            Ok(value) => value,
            Err(error) => {
                self.record_audit("error", "miss", "miss")?;
                return Err(error);
            }
        };
        let expr = checked.object().as_expr(cx)?;
        let mut cache_status = "disabled";
        if cache_enabled && cache_write_allowed(&self.card.policy.cache) {
            self.state.cache_store(key.clone(), expr.clone())?;
            cache_status = "stored";
        } else if cache_enabled && cache_read_allowed(&self.card.policy.cache) {
            cache_status = "miss";
        }
        let mut cassette_status = "disabled";
        if cassette_record_allowed(&self.card.policy.cassette) {
            self.state.cassette_store(key, expr)?;
            cassette_status = "stored";
        } else if cassette_replay_allowed(&self.card.policy.cassette) {
            cassette_status = "miss";
        }
        self.record_audit("live", cache_status, cassette_status)?;
        Ok(checked)
    }

    #[cfg(any(feature = "cache", feature = "cassette"))]
    fn record_audit(
        &self,
        outcome: &'static str,
        cache: &'static str,
        cassette: &'static str,
    ) -> Result<()> {
        let Some(registry) = &self.registry else {
            return Ok(());
        };
        let mut capabilities = self
            .card
            .capabilities
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect::<Vec<_>>();
        capabilities.sort();
        registry.record_audit(SkillAuditEntry {
            skill_id: self.card.id.clone(),
            transport_kind: self.card.transport_kind.clone(),
            outcome,
            cache,
            cassette,
            privacy: self.card.policy.privacy.as_symbol().to_string(),
            capabilities,
        })
    }

    fn check_args(&self, cx: &mut Cx, args: &[Value]) -> Result<Value> {
        let args_value = cx.factory().list(args.to_vec())?;
        let matched = check_value_report(cx, &self.card.input_shape, args_value.clone())?;
        if matched.accepted {
            Ok(args_value)
        } else {
            Err(Error::WrongShape {
                expected: shape_id(&self.card.input_shape),
                diagnostics: matched.diagnostics,
            })
        }
    }

    fn check_result(&self, cx: &mut Cx, value: Value) -> Result<Value> {
        let matched = check_value_report(cx, &self.card.output_shape, value.clone())?;
        if matched.accepted {
            Ok(value)
        } else {
            Err(Error::WrongShape {
                expected: shape_id(&self.card.output_shape),
                diagnostics: matched.diagnostics,
            })
        }
    }
}

impl Object for SkillCallable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<skill-callable {}>", self.card.id))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for SkillCallable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for SkillCallable {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.call_values(cx, args.into_vec(), None)
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(self.card.input_shape.clone()))
    }

    fn browse_result_shape(&self, _cx: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(self.card.output_shape.clone()))
    }
}

fn shape_id(shape: &ShapeRef) -> ShapeId {
    shape
        .object()
        .as_shape()
        .and_then(|shape| shape.id())
        .unwrap_or(ShapeId(0))
}
