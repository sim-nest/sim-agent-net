//! Remote and relayed surfaces over the kernel [`EvalFabric`].
//!
//! [`EvalFabric`] is location-transparent by design: a caller submits an
//! [`EvalRequest`] and receives an [`EvalReply`] without knowing whether the
//! work ran locally or across a network. A REMOTE or RELAYED surface is
//! therefore just another [`EvalFabric`] implementation that forwards to an
//! inner [`EvalFabricRef`] living on the far side of a link.
//!
//! [`RelayFabric`] models that far-side node -- for example a phone bridging a
//! watch's hardware to SIM. It is a CONNECTIVITY gate, not a resume engine. Its
//! load-bearing contract is twofold:
//!
//! - **Location transparency when connected.** When the link is up and policy
//!   allows the request, [`RelayFabric::realize`] returns exactly the inner
//!   fabric's reply.
//! - **No capability escalation.** A relay can only let through what its own
//!   policy already allows. A request that names a capability outside the
//!   relay's allowed set is refused fail-closed, naming the capability, and the
//!   inner fabric is never reached.
//!
//! # No resume; at-most-once per attempt
//!
//! The relay does NOT recover in-flight work across a link drop. Each
//! [`RelayFabric::realize`] call is a single attempt with at-most-once
//! semantics: it either reaches the inner fabric and returns its reply, or it
//! returns `Err`. A request whose link drops mid-flight returns an error and is
//! NOT auto-resumed or replayed; the caller decides whether to retry, and a
//! retry is a fresh attempt with no deduplication here. [`RelayFabric::reconnect`]
//! only restores the link to [`RelayStatus::Connected`] so that subsequent
//! attempts may proceed; it carries no session state, sequence, or idempotency
//! token. Idempotency and exactly-once delivery, if needed, are the far-side
//! fabric's responsibility, not the relay's.
//!
//! # Backpressure
//!
//! A relay carries a surface session's stream traffic over the crate's existing
//! bounded-frame discipline; it does not re-implement streaming. The frame codec
//! applies [`crate::StreamFrameLimits`], which caps in-flight frames, total
//! frames, payload size, duration, and rate. Once a bound is crossed the encoder
//! appends a limit diagnostic frame and TRUNCATES -- it stops emitting further
//! frames. It does NOT shed stale frames or keep the newest: there is no ring
//! buffer, so a lagging consumer sees a truncated stream with a diagnostic, not
//! a window of the most recent frames.

use std::collections::BTreeSet;

use sim_kernel::{
    CapabilityName, Cx, Error, EvalFabric, EvalFabricRef, EvalReply, EvalRequest, Result,
};

/// The connection state of a [`RelayFabric`] link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayStatus {
    /// The link is up; requests may be served.
    Connected,
    /// The link dropped and a reconnect is in progress; requests fail closed.
    Reconnecting,
    /// The link is severed; requests fail closed.
    Disconnected,
}

/// A remote or relayed [`EvalFabric`] node carrying a surface session.
///
/// `RelayFabric` wraps an inner [`EvalFabricRef`] reached across a link and
/// enforces a non-escalating capability policy in front of it. Because
/// [`EvalFabric`] is location-transparent, code that targets the `realize`
/// surface treats a relay exactly like a local fabric; the relay differs only
/// in that it can refuse and that its link can drop (failing closed, with no
/// resume).
///
/// The relay preserves each request's [`Consistency`](sim_kernel::Consistency)
/// unchanged and lets the inner fabric honor it: the inner node beyond the link
/// is the remote authority, so a relay boundary is naturally a
/// `RemoteOnly`/`LocalFirst` hop, but the relay never rewrites the caller's
/// declared consistency.
///
/// # Trust of declared capabilities
///
/// The capability gate matches each request's self-declared
/// [`EvalRequest::required_capabilities`] against the allowed set. The relay
/// therefore PRESUMES truthful capability declaration: it is a connectivity
/// gate, not the enforcement authority. A caller that under-declares its
/// capabilities can pass the relay; the inner / far-side fabric, which performs
/// the real operations behind its own `cx.require(...)` checks, is the actual
/// authority. The relay's gate exists to fail fast and avoid even reaching the
/// link for plainly out-of-policy requests, not to substitute for far-side
/// enforcement.
pub struct RelayFabric {
    inner: EvalFabricRef,
    allowed: BTreeSet<CapabilityName>,
    status: RelayStatus,
}

impl RelayFabric {
    /// Builds a connected relay over `inner` that allows only `allowed`.
    ///
    /// The relay starts [`RelayStatus::Connected`]. Any request whose
    /// [`EvalRequest::required_capabilities`] step outside `allowed` is refused
    /// rather than forwarded, so the relay can never grant more than it was
    /// configured to pass through.
    pub fn new(inner: EvalFabricRef, allowed: Vec<CapabilityName>) -> Self {
        Self {
            inner,
            allowed: allowed.into_iter().collect(),
            status: RelayStatus::Connected,
        }
    }

    /// Returns the relay's current link [`RelayStatus`].
    pub fn status(&self) -> RelayStatus {
        self.status
    }

    /// Reports whether `capability` is within the relay's allowed set.
    pub fn allows(&self, capability: &CapabilityName) -> bool {
        self.allowed.contains(capability)
    }

    /// Severs the link, moving the relay to [`RelayStatus::Disconnected`].
    ///
    /// While disconnected, [`RelayFabric::realize`] fails closed and never
    /// reaches the inner fabric.
    pub fn disconnect(&mut self) {
        self.status = RelayStatus::Disconnected;
    }

    /// Marks the link as [`RelayStatus::Reconnecting`].
    ///
    /// Requests still fail closed until [`RelayFabric::reconnect`] completes the
    /// handshake.
    pub fn begin_reconnect(&mut self) {
        self.status = RelayStatus::Reconnecting;
    }

    /// Restores the link to [`RelayStatus::Connected`].
    ///
    /// This only re-opens the gate so that subsequent [`RelayFabric::realize`]
    /// attempts may proceed. It carries no session state and does NOT resume any
    /// request that was in flight when the link dropped: such a request already
    /// returned `Err` and is the caller's to retry as a fresh attempt.
    pub fn reconnect(&mut self) {
        self.status = RelayStatus::Connected;
    }
}

impl EvalFabric for RelayFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        match self.status {
            RelayStatus::Connected => {}
            RelayStatus::Reconnecting => {
                return Err(Error::Eval(
                    "relay reconnecting: cannot realize until the link is restored".to_owned(),
                ));
            }
            RelayStatus::Disconnected => {
                return Err(Error::Eval(
                    "relay disconnected: cannot realize over a severed relay link".to_owned(),
                ));
            }
        }

        // Fail closed before delegating: a relay cannot escalate capabilities,
        // so a request whose self-declared required capabilities step outside
        // the allowed set is refused and the inner fabric is never reached. This
        // gate presumes truthful declaration; the far-side fabric is the real
        // authority (see the type docs).
        for capability in &request.required_capabilities {
            if !self.allowed.contains(capability) {
                return Err(Error::CapabilityDenied {
                    capability: capability.clone(),
                });
            }
        }

        self.inner.realize(cx, request)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sim_kernel::{
        CapabilityName, Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr,
        Result, Value,
    };

    use super::{RelayFabric, RelayStatus};

    /// An inner fabric that records the capabilities of every request it sees
    /// and answers with a fixed reply, like the kernel's `StaticFabric`.
    struct RecordingFabric {
        reply: EvalReply,
        seen: Mutex<Vec<Vec<CapabilityName>>>,
    }

    impl EvalFabric for RecordingFabric {
        fn realize(&self, _cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
            self.seen
                .lock()
                .expect("seen mutex")
                .push(request.required_capabilities.clone());
            Ok(self.reply.clone())
        }
    }

    use sim_kernel::testing::bare_cx as cx;

    fn recording(value: Value) -> Arc<RecordingFabric> {
        Arc::new(RecordingFabric {
            reply: EvalReply {
                value,
                diagnostics: Vec::new(),
                trace: None,
            },
            seen: Mutex::new(Vec::new()),
        })
    }

    fn request(caps: Vec<CapabilityName>) -> EvalRequest {
        EvalRequest {
            expr: Expr::Nil,
            result_shape: None,
            required_capabilities: caps,
            deadline: None,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        }
    }

    #[test]
    fn allowed_request_passes_through_to_inner_reply() {
        let mut cx = cx();
        let value = cx.factory().string("ok".to_owned()).unwrap();
        let cap = CapabilityName::new("relay.allowed");
        let inner = recording(value.clone());
        let relay = RelayFabric::new(inner.clone(), vec![cap.clone()]);

        let reply = relay.realize(&mut cx, request(vec![cap.clone()])).unwrap();

        // Location transparency: the relay returns exactly the inner reply.
        assert_eq!(reply.value, value);
        assert!(reply.diagnostics.is_empty());
        let seen = inner.seen.lock().unwrap();
        assert_eq!(seen.as_slice(), &[vec![cap]]);
    }

    #[test]
    fn relay_has_no_caching_layer() {
        let mut cx = cx();
        let allowed = CapabilityName::new("relay.allowed");
        let value = cx.factory().string("no-cache".to_owned()).unwrap();
        let inner = recording(value);
        let relay = RelayFabric::new(inner.clone(), vec![allowed.clone()]);

        relay
            .realize(&mut cx, request(vec![allowed.clone()]))
            .unwrap();
        relay.realize(&mut cx, request(vec![allowed])).unwrap();

        assert_eq!(
            inner.seen.lock().unwrap().len(),
            2,
            "relay must not cache; every call must reach inner"
        );
    }

    #[test]
    fn refused_capability_fails_closed_without_reaching_inner() {
        let mut cx = cx();
        let value = cx.factory().nil().unwrap();
        let allowed = CapabilityName::new("relay.allowed");
        let refused = CapabilityName::new("relay.refused");
        let inner = recording(value);
        let relay = RelayFabric::new(inner.clone(), vec![allowed]);

        let err = relay
            .realize(&mut cx, request(vec![refused.clone()]))
            .err()
            .expect("refused request must fail closed");

        match err {
            Error::CapabilityDenied { capability } => assert_eq!(capability, refused),
            other => panic!("expected CapabilityDenied naming the refused capability, got {other}"),
        }
        // No escalation: the inner fabric was never asked to realize anything.
        assert!(inner.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn disconnect_fails_closed_then_reconnect_restores_connectivity() {
        let mut cx = cx();
        let value = cx.factory().string("reconnected".to_owned()).unwrap();
        let cap = CapabilityName::new("relay.allowed");
        let inner = recording(value.clone());
        let mut relay = RelayFabric::new(inner.clone(), vec![cap.clone()]);

        relay.disconnect();
        assert_eq!(relay.status(), RelayStatus::Disconnected);
        let err = relay
            .realize(&mut cx, request(vec![cap.clone()]))
            .err()
            .expect("disconnected relay must fail closed");
        assert!(matches!(err, Error::Eval(message) if message.contains("relay disconnected")));
        // No resume: the dropped attempt never reached the inner fabric.
        assert!(inner.seen.lock().unwrap().is_empty());

        relay.begin_reconnect();
        assert_eq!(relay.status(), RelayStatus::Reconnecting);
        // Still fails closed mid-handshake.
        assert!(relay.realize(&mut cx, request(vec![cap.clone()])).is_err());

        // Reconnect only re-opens the gate; it carries no token or session state.
        relay.reconnect();
        assert_eq!(relay.status(), RelayStatus::Connected);
        let reply = relay.realize(&mut cx, request(vec![cap.clone()])).unwrap();
        assert_eq!(reply.value, value);

        // A second drop + reconnect cycle simply restores connectivity again;
        // each realize is an independent at-most-once attempt.
        relay.disconnect();
        relay.begin_reconnect();
        relay.reconnect();
        assert_eq!(relay.status(), RelayStatus::Connected);
        relay.realize(&mut cx, request(vec![cap])).unwrap();
        // Two successful attempts reached the inner fabric -- no dedup, no double
        // suppression, exactly the two attempts the caller made.
        assert_eq!(inner.seen.lock().unwrap().len(), 2);
    }
}
