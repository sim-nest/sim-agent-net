//! Modern request-owned state and explicit shared-provider contracts.

use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use sim_cancel::Cancellation;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_protected_state::{ConsumptionLedger, ProtectedState, StateBinding};

use crate::RequestContext;

/// Modern operations eligible to suspend for caller input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Invoke a tool.
    ToolsCall,
    /// Fetch a prompt.
    PromptsGet,
    /// Read a resource.
    ResourcesRead,
}

/// Versioned, caller-serialized MRTR continuation payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpContinuation {
    /// Payload format version.
    pub version: u8,
    /// Suspended operation.
    pub operation: OperationKind,
    /// Digest of canonical parameters excluding response fields.
    pub parameter_digest: String,
    /// Authenticated principal identifier.
    pub principal_id: String,
    /// Digest of the diminished capability set.
    pub admitted_capability_digest: String,
    /// Inputs still required, keyed by request name and valued by capability.
    pub requested_inputs: BTreeMap<String, String>,
    /// Inclusive issue instant in Unix milliseconds.
    pub issued_at_ms: u64,
    /// Exclusive expiry instant in Unix milliseconds.
    pub expires_at_ms: u64,
}

impl McpContinuation {
    /// Current payload version.
    pub const VERSION: u8 = 1;

    /// Protects the canonical JSON payload under the exact MCP binding.
    pub fn protect(&self, state: &ProtectedState, audience: &str) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| Error::Eval(error.to_string()))?;
        state
            .protect(&bytes, &self.binding(audience)?)
            .map_err(|error| Error::Eval(error.to_string()))
    }

    /// Opens and validates an opaque payload. The caller must then re-run the
    /// normal authorization pipeline against the current request.
    pub fn open(
        state: &ProtectedState,
        opaque: &[u8],
        expected: &Self,
        audience: &str,
    ) -> Result<Self> {
        let bytes = state
            .open(opaque, &expected.binding(audience)?)
            .map_err(|error| Error::Eval(error.to_string()))?;
        let opened: Self = serde_json::from_slice(bytes.expose())
            .map_err(|error| Error::Eval(error.to_string()))?;
        opened.validate()?;
        if opened.operation != expected.operation
            || opened.parameter_digest != expected.parameter_digest
            || opened.principal_id != expected.principal_id
            || opened.admitted_capability_digest != expected.admitted_capability_digest
        {
            return Err(Error::Eval(
                "MCP continuation authorization mismatch".into(),
            ));
        }
        Ok(opened)
    }

    /// Removes exactly those requested inputs answered by the caller. Unknown
    /// response names are ignored; absent inputs keep the request suspended.
    pub fn apply_responses(&mut self, responses: &BTreeMap<String, Expr>) {
        self.requested_inputs
            .retain(|name, _| !responses.contains_key(name));
    }

    /// Claims a harmful replay only when the operation's injected policy asks
    /// for single use. Idempotent reads simply omit the ledger.
    pub fn claim_once(&self, cx: &mut Cx, ledger: Option<&dyn ConsumptionLedger>) -> Result<bool> {
        let Some(ledger) = ledger else {
            return Ok(true);
        };
        ledger
            .claim(
                cx,
                Symbol::qualified("mcp-continuation", self.parameter_digest.as_str()),
            )
            .map_err(|error| Error::Eval(error.to_string()))
    }

    fn validate(&self) -> Result<()> {
        if self.version != Self::VERSION || self.issued_at_ms >= self.expires_at_ms {
            return Err(Error::Eval("invalid MCP continuation payload".into()));
        }
        Ok(())
    }

    fn binding(&self, audience: &str) -> Result<StateBinding> {
        StateBinding::new(
            b"mcp/request-state/v1".to_vec(),
            audience.as_bytes().to_vec(),
            self.principal_id.as_bytes().to_vec(),
            format!(
                "{}:{}",
                self.parameter_digest, self.admitted_capability_digest
            )
            .into_bytes(),
            self.expires_at_ms,
        )
        .map_err(|error| Error::Eval(error.to_string()))
    }
}

/// Documented synchronization behavior of an injected provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderConcurrency {
    /// Every operation is safe to call concurrently.
    Concurrent,
    /// The provider serializes its own operations internally.
    Serialized,
}

/// Explicit shared durable application service. Implementations own their
/// construction-time authority and synchronization; MCP never discovers one
/// from ambient state.
pub trait DurableProvider: Send + Sync {
    /// Concurrency contract promised by this provider.
    fn concurrency(&self) -> ProviderConcurrency;
}

/// One immutable event from an injected application event source.
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionEvent {
    /// Event kind used by the exact subscription filter.
    pub kind: String,
    /// Semantic event body.
    pub body: Expr,
}

/// One ordered item emitted by a modern subscription.
#[derive(Clone, Debug, PartialEq)]
pub enum SubscriptionMessage {
    /// First item, confirming the exact filter accepted by the source.
    Acknowledged {
        /// Subscription identifier.
        subscription_id: String,
        /// Exact filter honored by the source.
        honored_filter: Vec<String>,
    },
    /// One event carrying the subscription identifier on the wire.
    Event {
        /// Subscription identifier.
        subscription_id: String,
        /// Event supplied by the injected source.
        event: SubscriptionEvent,
    },
    /// Final item, emitted for normal exhaustion or cancellation.
    Complete {
        /// Subscription identifier.
        subscription_id: String,
        /// Host-supplied completion instant in Unix milliseconds.
        completed_at_ms: u64,
        /// Whether cancellation caused teardown.
        cancelled: bool,
    },
}

/// Explicit shared event source.
pub trait EventProvider: Send + Sync {
    /// Concurrency contract promised by this provider.
    fn concurrency(&self) -> ProviderConcurrency;
    /// Returns a bounded snapshot honoring `filter` exactly.
    fn events(&self, filter: &[String], limit: usize) -> Result<Vec<SubscriptionEvent>>;
}

/// Construction-time provider catalog. All fields are explicit and immutable;
/// the providers themselves own any deliberately shared durable state.
#[derive(Default)]
pub struct McpProviders {
    /// Named durable Table/store/model services.
    pub durable: BTreeMap<String, Arc<dyn DurableProvider>>,
    /// Named event sources.
    pub events: BTreeMap<String, Arc<dyn EventProvider>>,
    /// Optional protected continuation carrier.
    pub protected_state: Option<Arc<ProtectedState>>,
}

/// One bounded subscription request. It owns fresh cancellation and never
/// shares queue state with another subscription.
pub struct McpSubscription {
    id: String,
    filter: Vec<String>,
    maximum_events: usize,
    cancellation: Cancellation,
}

impl McpSubscription {
    /// Creates an independent subscription with an exact filter and queue cap.
    pub fn new(id: impl Into<String>, filter: Vec<String>, maximum_events: usize) -> Result<Self> {
        if maximum_events == 0 {
            return Err(Error::Eval(
                "subscription queue must be bounded above zero".into(),
            ));
        }
        Ok(Self {
            id: id.into(),
            filter,
            maximum_events,
            cancellation: Cancellation::new(),
        })
    }
    /// Subscription id copied onto every acknowledgement, event, and terminal.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Exact honored filter reported by the acknowledgement.
    pub fn honored_filter(&self) -> &[String] {
        &self.filter
    }
    /// Request-owned cancellation token propagated to its source adapter.
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
    /// Reads the bounded event sequence; callers serialize acknowledgement
    /// first, then these events, then a dated terminal/complete message.
    pub fn read(&self, source: &dyn EventProvider) -> Result<Vec<SubscriptionEvent>> {
        if self.cancellation.is_cancelled() {
            return Ok(Vec::new());
        }
        source.events(&self.filter, self.maximum_events)
    }

    /// Produces the complete bounded wire sequence: acknowledgement first,
    /// zero or more identified events, and one dated terminal item last.
    pub fn sequence(
        &self,
        source: &dyn EventProvider,
        completed_at_ms: u64,
    ) -> Result<Vec<SubscriptionMessage>> {
        let mut messages = vec![SubscriptionMessage::Acknowledged {
            subscription_id: self.id.clone(),
            honored_filter: self.filter.clone(),
        }];
        let cancelled = self.cancellation.is_cancelled();
        if !cancelled {
            messages.extend(self.read(source)?.into_iter().map(|event| {
                SubscriptionMessage::Event {
                    subscription_id: self.id.clone(),
                    event,
                }
            }));
        }
        messages.push(SubscriptionMessage::Complete {
            subscription_id: self.id.clone(),
            completed_at_ms,
            cancelled,
        });
        Ok(messages)
    }
}

/// Validates that an input-required result is legal for this operation and
/// contains only capabilities declared by the current request.
pub fn validate_input_required(
    operation: OperationKind,
    context: &RequestContext,
    requested: &BTreeMap<String, String>,
) -> Result<()> {
    let _eligible = operation;
    if requested.is_empty()
        || requested
            .values()
            .any(|capability| !context.input_capabilities().contains(capability))
    {
        return Err(Error::Eval("undeclared MCP input capability".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_cancel::CancellationReason;
    use std::sync::Mutex;

    struct SharedEvents(Mutex<Vec<SubscriptionEvent>>);
    impl EventProvider for SharedEvents {
        fn concurrency(&self) -> ProviderConcurrency {
            ProviderConcurrency::Serialized
        }
        fn events(&self, filter: &[String], limit: usize) -> Result<Vec<SubscriptionEvent>> {
            Ok(self
                .0
                .lock()
                .map_err(|_| Error::PoisonedLock("shared events"))?
                .iter()
                .filter(|event| filter.contains(&event.kind))
                .take(limit)
                .cloned()
                .collect())
        }
    }

    #[test]
    fn subscriptions_are_independent_bounded_and_honor_exact_filters() {
        let source = SharedEvents(Mutex::new(vec![
            SubscriptionEvent {
                kind: "a".into(),
                body: Expr::String("one".into()),
            },
            SubscriptionEvent {
                kind: "b".into(),
                body: Expr::String("private".into()),
            },
            SubscriptionEvent {
                kind: "a".into(),
                body: Expr::String("two".into()),
            },
        ]));
        let first = McpSubscription::new("s1", vec!["a".into()], 1).unwrap();
        let second = McpSubscription::new("s2", vec!["b".into()], 4).unwrap();
        assert_eq!(first.read(&source).unwrap().len(), 1);
        assert_eq!(second.read(&source).unwrap().len(), 1);
        assert_ne!(first.id(), second.id());
        assert_eq!(first.honored_filter(), &["a"]);
        let messages = first.sequence(&source, 42).unwrap();
        assert!(matches!(
            messages.first(),
            Some(SubscriptionMessage::Acknowledged {
                subscription_id,
                honored_filter,
            }) if subscription_id == "s1" && honored_filter == &["a"]
        ));
        assert!(matches!(
            messages.get(1),
            Some(SubscriptionMessage::Event { subscription_id, .. })
                if subscription_id == "s1"
        ));
        assert!(matches!(
            messages.last(),
            Some(SubscriptionMessage::Complete {
                subscription_id,
                completed_at_ms: 42,
                cancelled: false,
            }) if subscription_id == "s1"
        ));

        second.cancellation().cancel(
            CancellationReason::new("subscription test cancellation")
                .expect("static cancellation reason is valid"),
        );
        assert!(matches!(
            second.sequence(&source, 43).unwrap().as_slice(),
            [
                SubscriptionMessage::Acknowledged { .. },
                SubscriptionMessage::Complete {
                    completed_at_ms: 43,
                    cancelled: true,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn mrtr_ignores_unexpected_responses_and_reissues_remaining_inputs() {
        let mut continuation = McpContinuation {
            version: McpContinuation::VERSION,
            operation: OperationKind::ToolsCall,
            parameter_digest: "params".into(),
            principal_id: "alice".into(),
            admitted_capability_digest: "caps".into(),
            requested_inputs: BTreeMap::from([
                ("sample".into(), "sampling".into()),
                ("root".into(), "roots".into()),
            ]),
            issued_at_ms: 10,
            expires_at_ms: 20,
        };
        continuation.apply_responses(&BTreeMap::from([
            ("sample".into(), Expr::String("ok".into())),
            ("unexpected".into(), Expr::String("ignored".into())),
        ]));
        assert_eq!(
            continuation.requested_inputs,
            BTreeMap::from([("root".into(), "roots".into())])
        );
    }
}
