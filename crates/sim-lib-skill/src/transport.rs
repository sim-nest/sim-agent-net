use std::sync::Arc;

use sim_citizen_derive::non_citizen;
use sim_kernel::{Cx, Expr, Object, ObjectCompat, Result, Symbol, Value};

use crate::SkillCard;

/// Sink that receives streaming events emitted during a skill call.
pub trait SkillEventSink {
    /// Emits one `event` produced while a skill call is in progress.
    fn emit(&mut self, cx: &mut Cx, event: Value) -> Result<()>;
}

/// Backend that discovers and runs skills.
///
/// A transport is the concrete behavior behind a [`SkillCard`]: it knows how
/// to enumerate available skills and how to dispatch a call for one.
/// Implementations include the in-process [`FixtureTransport`] and the
/// feature-gated MCP, HTTP, process, and OpenAI-server transports.
///
/// [`FixtureTransport`]: crate::FixtureTransport
pub trait SkillTransport: Send + Sync {
    /// Returns the transport's stable identifier.
    fn id(&self) -> &str;
    /// Returns the transport's kind (for example `fixture`, `mcp`, `http`).
    fn kind(&self) -> &str;
    /// Discovers the skills this transport can run, as cards.
    fn discover(&self, cx: &mut Cx) -> Result<Vec<SkillCard>>;
    /// Runs the skill described by `card` with the given `args`.
    ///
    /// The optional `events` sink receives any streaming events emitted while
    /// the call is in progress.
    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value>;
    /// Returns a value describing the transport's health.
    fn health(&self, cx: &mut Cx) -> Result<Value>;
}

/// Runtime [`Object`] wrapper around a shared [`SkillTransport`].
///
/// This is the object passed to `skill/install` to register a transport. It is
/// a live handle; its transport metadata is also carried by the `skill/Card`
/// descriptor.
#[derive(Clone)]
#[non_citizen(
    reason = "live skill transport handle; transport metadata is carried by skill/Card descriptor",
    kind = "handle"
)]
pub struct SkillTransportValue {
    transport: Arc<dyn SkillTransport>,
}

impl SkillTransportValue {
    /// Wraps `transport` in a runtime value.
    pub fn new(transport: Arc<dyn SkillTransport>) -> Self {
        Self { transport }
    }

    /// Returns a shared handle to the wrapped transport.
    pub fn transport(&self) -> Arc<dyn SkillTransport> {
        self.transport.clone()
    }
}

impl Object for SkillTransportValue {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!(
            "#<skill-transport {}:{}>",
            self.transport.kind(),
            self.transport.id()
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for SkillTransportValue {
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/transport"))?,
            ),
            (
                Symbol::new("id"),
                cx.factory().string(self.transport.id().to_owned())?,
            ),
            (
                Symbol::new("transport-kind"),
                cx.factory().string(self.transport.kind().to_owned())?,
            ),
        ])
    }
}

/// Wraps `transport` in an opaque [`SkillTransportValue`] runtime value.
pub fn skill_transport_value(cx: &mut Cx, transport: Arc<dyn SkillTransport>) -> Result<Value> {
    cx.factory()
        .opaque(Arc::new(SkillTransportValue::new(transport)))
}
