use sim_codec_bridge::{
    BridgeCallArgument, BridgeCallPayload, BridgePacket, CallArgumentMedia, content_id_string,
    packet_content_id, stamp_packet_cid,
};
use sim_kernel::{CapabilityName, Cx, Datum, DatumStore, Error, EvalFabric, Expr, Result, Symbol};
use sim_lib_agent_runner_core::InjectionFence;
use sim_lib_bridge::{RepairPolicy, effective_caps, run_ask_with_policy};

use crate::{CompiledIntent, VerifyCatalog, packet_artifact::resolve_packet};

/// One eval target in a cost-ordered route ladder.
pub struct RouteTarget<'a> {
    /// Stable route id recorded in provenance and attempt reports.
    pub id: String,
    /// Eval fabric used for this target.
    pub fabric: &'a dyn EvalFabric,
    /// Capabilities this concrete target requires in addition to the packet
    /// request capabilities.
    pub required_capabilities: Vec<Symbol>,
    /// Optional model or placement card recorded when this target answers.
    pub card: Option<String>,
}

impl<'a> RouteTarget<'a> {
    /// Builds a route target with no extra target capability requirement.
    pub fn new(id: impl Into<String>, fabric: &'a dyn EvalFabric) -> Self {
        Self {
            id: id.into(),
            fabric,
            required_capabilities: Vec::new(),
            card: None,
        }
    }

    /// Returns this target with explicit capability requirements.
    pub fn requiring(mut self, required_capabilities: Vec<Symbol>) -> Self {
        self.required_capabilities = required_capabilities;
        self
    }

    /// Returns this target with a model or placement card descriptor.
    pub fn with_card(mut self, card: impl Into<String>) -> Self {
        self.card = Some(card.into());
        self
    }
}

/// Cost-aware route policy for a compiled intent call.
pub struct RoutePolicy<'a> {
    /// Cheap-first ordered ladder of eval targets.
    pub ladder: Vec<RouteTarget<'a>>,
    /// Number of terminal structural or semantic failures allowed on a target
    /// before the router escalates to the next target.
    pub escalate_after: u8,
    /// BRIDGE bounded-repair retry count used inside one target before a
    /// structural failure is counted.
    pub repair_retries: u8,
    /// Semantic verifier catalog that checks decoded answers.
    pub verify_catalog: VerifyCatalog,
}

impl<'a> RoutePolicy<'a> {
    /// Builds a route policy with default one-shot escalation and BRIDGE repair.
    pub fn new(ladder: Vec<RouteTarget<'a>>, escalate_after: u8) -> Self {
        Self {
            ladder,
            escalate_after: escalate_after.max(1),
            repair_retries: RepairPolicy::default().max_retries,
            verify_catalog: VerifyCatalog::new(),
        }
    }

    /// Returns this policy with an explicit repair retry count.
    pub fn with_repair_retries(mut self, repair_retries: u8) -> Self {
        self.repair_retries = repair_retries.min(2);
        self
    }

    /// Returns this policy with an explicit semantic verifier catalog.
    pub fn with_verify_catalog(mut self, verify_catalog: VerifyCatalog) -> Self {
        self.verify_catalog = verify_catalog;
        self
    }

    fn repair_policy(&self) -> RepairPolicy {
        RepairPolicy::new(self.repair_retries)
    }
}

/// Status for one route attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteAttemptStatus {
    /// The target was skipped before any model call.
    Skipped,
    /// The target produced a structural or semantic failure.
    Failed,
    /// The target answered and passed every required check.
    Accepted,
}

/// One target attempt made by the router.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteAttempt {
    /// Target id attempted.
    pub target: String,
    /// Attempt status.
    pub status: RouteAttemptStatus,
    /// Structural or semantic reason, when the attempt did not answer.
    pub reason: Option<String>,
}

/// Provenance for the routed answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteProvenance {
    /// Target id that produced the accepted answer.
    pub target: String,
    /// Optional model or placement card descriptor for the answering target.
    pub card: Option<String>,
    /// Content id string of the checked answer packet.
    pub answer_packet: String,
}

/// Answer plus route provenance and attempt details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedAnswer {
    /// Decoded answer expression from the accepted BRIDGE reply packet.
    pub answer: Expr,
    /// Provenance for the accepted answer.
    pub provenance: RouteProvenance,
    /// Ordered attempts made before acceptance.
    pub attempts: Vec<RouteAttempt>,
}

/// Runs a compiled intent through a cost-aware route ladder and returns only
/// the accepted decoded answer.
pub fn run_intent_routed(
    cx: &mut Cx,
    intent: &CompiledIntent,
    args: &Expr,
    policy: &RoutePolicy<'_>,
) -> Result<Expr> {
    Ok(run_intent_routed_report(cx, intent, args, policy)?.answer)
}

/// Runs a compiled intent through a cost-aware route ladder and returns route
/// provenance with the accepted decoded answer.
pub fn run_intent_routed_report(
    cx: &mut Cx,
    intent: &CompiledIntent,
    args: &Expr,
    policy: &RoutePolicy<'_>,
) -> Result<RoutedAnswer> {
    if policy.ladder.is_empty() {
        return Err(Error::Eval("route policy ladder is empty".to_owned()));
    }

    let packet = resolve_packet(cx, &intent.packet)?;
    let executable = bind_args(cx, &packet, args)?;
    let mut attempts = Vec::new();

    for target in &policy.ladder {
        if let Some(reason) = skip_reason(cx, &executable, target)? {
            attempts.push(RouteAttempt {
                target: target.id.clone(),
                status: RouteAttemptStatus::Skipped,
                reason: Some(reason),
            });
            continue;
        }

        let mut failures = 0u8;
        while failures < policy.escalate_after {
            match run_target_once(cx, intent, &executable, target, policy) {
                Ok((answer, answer_packet)) => {
                    let packet_cid = answer_packet.header.cid.clone().unwrap_or_else(|| {
                        packet_content_id(&answer_packet)
                            .map(|id| content_id_string(&id))
                            .unwrap_or_else(|_| "unhashable".to_owned())
                    });
                    attempts.push(RouteAttempt {
                        target: target.id.clone(),
                        status: RouteAttemptStatus::Accepted,
                        reason: None,
                    });
                    return Ok(RoutedAnswer {
                        answer,
                        provenance: RouteProvenance {
                            target: target.id.clone(),
                            card: target
                                .card
                                .clone()
                                .or_else(|| answer_packet.header.provenance.card.clone()),
                            answer_packet: packet_cid,
                        },
                        attempts,
                    });
                }
                Err(reason) => {
                    failures = failures.saturating_add(1);
                    attempts.push(RouteAttempt {
                        target: target.id.clone(),
                        status: RouteAttemptStatus::Failed,
                        reason: Some(reason),
                    });
                }
            }
        }
    }

    Err(Error::Eval(format!(
        "routed intent {} exhausted {} target(s): {}",
        intent.name,
        policy.ladder.len(),
        attempt_summary(&attempts)
    )))
}

fn run_target_once(
    cx: &mut Cx,
    intent: &CompiledIntent,
    packet: &BridgePacket,
    target: &RouteTarget<'_>,
    policy: &RoutePolicy<'_>,
) -> std::result::Result<(Expr, BridgePacket), String> {
    let answer_packet =
        run_ask_with_policy(cx, target.fabric, packet.clone(), policy.repair_policy())
            .map_err(|err| format!("structural check failed: {err}"))?;
    let answer = answer_from_packet(&answer_packet).map_err(|err| err.to_string())?;
    let verify_report = policy
        .verify_catalog
        .verify_answer(cx, intent, &answer)
        .map_err(|err| err.to_string())?;
    if verify_report.accepted() {
        Ok((answer, answer_packet))
    } else {
        Err(format!(
            "semantic verification failed: {:?}",
            verify_report.failed
        ))
    }
}

fn answer_from_packet(packet: &BridgePacket) -> Result<Expr> {
    packet
        .body
        .iter()
        .find(|part| part.id == packet.header.output)
        .map(|part| part.payload.clone())
        .ok_or_else(|| {
            Error::Eval(format!(
                "answer packet output part {} is missing",
                packet.header.output
            ))
        })
}

fn skip_reason(
    cx: &mut Cx,
    packet: &BridgePacket,
    target: &RouteTarget<'_>,
) -> Result<Option<String>> {
    let allowed = effective_caps(cx, packet)?;
    for required in &target.required_capabilities {
        let capability = capability_name(required);
        if !allowed.contains(&capability) {
            return Ok(Some(format!(
                "target requires capability {capability}, outside packet ceiling"
            )));
        }
    }
    Ok(None)
}

fn bind_args(cx: &mut Cx, packet: &BridgePacket, args: &Expr) -> Result<BridgePacket> {
    let mut bound = packet.canonicalized();
    for part in &mut bound.body {
        if part.kind != Symbol::qualified("bridge", "Call") {
            continue;
        }
        let payload = BridgeCallPayload::from_expr(&part.payload)?.with_arg(fenced_arg(cx, args)?);
        part.payload = payload.to_expr();
        return stamp_packet_cid(&bound);
    }
    Err(Error::Eval(
        "routed intent packet requires a bridge/Call part".to_owned(),
    ))
}

fn fenced_arg(cx: &mut Cx, expr: &Expr) -> Result<BridgeCallArgument> {
    let text = sim_codec_json::expr_to_json(expr).to_string();
    let datum = Datum::String(text.clone());
    let content_id = cx.datum_store_mut().intern(datum)?;
    let fence = InjectionFence::for_content(&content_id);
    Ok(BridgeCallArgument::new(
        Symbol::qualified("forge", "args"),
        Symbol::qualified("codec", "json"),
        CallArgumentMedia::Text,
        content_id_string(&content_id),
        fence.wrap("forge-args", &text),
    ))
}

fn capability_name(symbol: &Symbol) -> CapabilityName {
    match symbol.namespace.as_deref() {
        Some("capability") => CapabilityName::new(symbol.name.to_string()),
        _ => CapabilityName::new(symbol.as_qualified_str()),
    }
}

fn attempt_summary(attempts: &[RouteAttempt]) -> String {
    if attempts.is_empty() {
        return "no attempts".to_owned();
    }
    attempts
        .iter()
        .map(|attempt| {
            let status = match attempt.status {
                RouteAttemptStatus::Skipped => "skipped",
                RouteAttemptStatus::Failed => "failed",
                RouteAttemptStatus::Accepted => "accepted",
            };
            match &attempt.reason {
                Some(reason) => format!("{} {status}: {reason}", attempt.target),
                None => format!("{} {status}", attempt.target),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}
