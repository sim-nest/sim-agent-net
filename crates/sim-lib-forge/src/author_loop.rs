use std::collections::BTreeSet;

use sim_codec::{Input, decode_with_codec};
use sim_kernel::{
    CapabilityName, CapabilitySet, Consistency, Cx, Diagnostic, Error, EvalMode, EvalReply,
    EvalRequest, Expr, ReadPolicy, Result, Symbol,
};
use sim_lib_agent_runner_core::{ModelResponse, OutputContract, terminal_model_content};
use sim_lib_stream_core::{DevCassette, DevEvent};
use sim_value::{access::field, build::entry};

use crate::{
    AuthorTask, CompiledIntent, IntentStatus, RankedContractCard, RouteAttempt, RouteAttemptStatus,
    RoutePolicy, RouteTarget,
    author::{author_model_request, project_contracts_with_cards},
};

/// Result of a contract-native authoring run.
pub struct AuthorOutcome {
    /// Decoded form that passed codec and Shape checks before realization.
    pub checked_form: Option<Expr>,
    /// Realized answer that passed the derived capability ceiling and verifiers.
    pub realized: Option<Expr>,
    /// Cheap-first route attempts made while authoring the task.
    pub attempts: Vec<RouteAttempt>,
    /// Non-fatal projection, routing, decoding, Shape, capability, and verifier diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Developer cassette containing the author-loop route and validation events.
    pub cassette: DevCassette,
}

/// Returns the sorted union of capabilities authorized by projected cards.
pub fn authorized_capabilities(projection_cards: &[RankedContractCard]) -> Vec<Symbol> {
    projection_cards
        .iter()
        .flat_map(|ranked| ranked.card.capability_symbols.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Runs one contract-native authoring task through routing, checks, realization, and cassette capture.
pub fn run_author_task(
    cx: &mut Cx,
    task: &AuthorTask,
    policy: &RoutePolicy<'_>,
) -> Result<AuthorOutcome> {
    let (projection, projection_cards) =
        project_contracts_with_cards(&task.contract_cards, &task.projection_caps);
    let mut diagnostics = projection.diagnostics.clone();
    let ceiling = authorized_capabilities(&projection_cards);
    let model_request = match author_model_request(cx, task, &projection) {
        Ok(request) => request,
        Err(err) => {
            diagnostics.push(Diagnostic::info(err.to_string()));
            return author_outcome(task, None, None, Vec::new(), diagnostics);
        }
    };

    let mut attempts = Vec::new();
    if policy.ladder.is_empty() {
        diagnostics.push(Diagnostic::info(format!(
            "author task {} has no route targets",
            task.name
        )));
        return author_outcome(task, None, None, attempts, diagnostics);
    }

    for target in &policy.ladder {
        if let Some(reason) = target_skip_reason(target, &ceiling) {
            attempts.push(RouteAttempt {
                target: target.id.clone(),
                status: RouteAttemptStatus::Skipped,
                reason: Some(reason),
            });
            continue;
        }

        for _ in 0..policy.escalate_after {
            match run_target_once(cx, task, policy, target, &model_request, &ceiling) {
                Ok(accepted) => {
                    diagnostics.extend(accepted.diagnostics);
                    attempts.push(RouteAttempt {
                        target: target.id.clone(),
                        status: RouteAttemptStatus::Accepted,
                        reason: None,
                    });
                    return author_outcome(
                        task,
                        Some(accepted.checked_form),
                        Some(accepted.realized),
                        attempts,
                        diagnostics,
                    );
                }
                Err(reason) => {
                    attempts.push(RouteAttempt {
                        target: target.id.clone(),
                        status: RouteAttemptStatus::Failed,
                        reason: Some(reason),
                    });
                }
            }
        }
    }

    diagnostics.push(Diagnostic::info(format!(
        "author task {} exhausted route policy without a checked form",
        task.name
    )));
    author_outcome(task, None, None, attempts, diagnostics)
}

struct AcceptedAuthorRun {
    checked_form: Expr,
    realized: Expr,
    diagnostics: Vec<Diagnostic>,
}

fn run_target_once(
    cx: &mut Cx,
    task: &AuthorTask,
    policy: &RoutePolicy<'_>,
    target: &RouteTarget<'_>,
    model_request: &sim_lib_agent_runner_core::ModelRequest,
    ceiling: &[Symbol],
) -> std::result::Result<AcceptedAuthorRun, String> {
    let model_reply = target
        .fabric
        .realize(cx, model_eval_request(model_request.clone()))
        .map_err(|err| format!("model request failed: {err}"))?;
    let response = decode_model_response(cx, model_reply)
        .map_err(|err| format!("model response failed: {err}"))?;
    let checked_form = decode_terminal_form(cx, task, &response)
        .map_err(|err| format!("terminal output failed: {err}"))?;
    let required = required_capabilities_for_form(&checked_form);
    if let Some(reason) = outside_ceiling_reason(&required, ceiling) {
        return Err(reason);
    }

    let narrowed = diminished_capabilities(cx.capabilities(), ceiling);
    let realize_request = realize_eval_request(checked_form.clone(), &required);
    let EvalReply {
        value, diagnostics, ..
    } = cx
        .with_capabilities(narrowed, |scoped| {
            target.fabric.realize(scoped, realize_request)
        })
        .map_err(|err| format!("realize failed: {err}"))?;
    let realized = value
        .object()
        .as_expr(cx)
        .map_err(|err| format!("realize value failed: {err}"))?;
    let matched = task
        .return_shape
        .check_expr(cx, &realized)
        .map_err(|err| format!("realized shape check failed: {err}"))?;
    if !matched.accepted {
        return Err(format!(
            "realized form failed return Shape: {}",
            shape_diagnostics(&matched.diagnostics)
        ));
    }

    let verify_report = policy
        .verify_catalog
        .verify_answer(cx, &author_intent(task), &realized)
        .map_err(|err| format!("verifier check failed: {err}"))?;
    if !verify_report.accepted() {
        let reasons = verify_report
            .failed
            .iter()
            .map(|failure| format!("{}: {}", failure.id, failure.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("verifier check failed: {reasons}"));
    }

    Ok(AcceptedAuthorRun {
        checked_form,
        realized,
        diagnostics,
    })
}

fn decode_model_response(cx: &mut Cx, reply: EvalReply) -> Result<ModelResponse> {
    ModelResponse::try_from(reply.value.object().as_expr(cx)?)
}

fn decode_terminal_form(cx: &mut Cx, task: &AuthorTask, response: &ModelResponse) -> Result<Expr> {
    let output = OutputContract::for_shape(
        task.target_codec.clone(),
        task.return_shape_expr.clone(),
        task.return_shape.as_ref(),
        task.strict_grammar,
    );
    let input = match terminal_model_content(response) {
        Ok(Expr::String(text)) => Input::Text(text.clone()),
        Ok(Expr::Bytes(bytes)) => Input::Bytes(bytes.clone()),
        Ok(Expr::Map(_)) => match field(terminal_model_content(response)?, "text") {
            Some(Expr::String(text)) => Input::Text(text.clone()),
            _ => {
                return Err(Error::Eval(
                    "terminal content map must carry text".to_owned(),
                ));
            }
        },
        Ok(other) => {
            return Err(Error::Eval(format!(
                "terminal content must be text or bytes, found {other:?}"
            )));
        }
        Err(err) => return Err(err),
    };
    let decoded = decode_with_codec(cx, &output.codec, input, ReadPolicy::default())?;
    let matched = task.return_shape.check_expr(cx, &decoded)?;
    if !matched.accepted {
        return Err(Error::Eval(format!(
            "decoded form failed return Shape: {}",
            shape_diagnostics(&matched.diagnostics)
        )));
    }
    Ok(decoded)
}

fn model_eval_request(model_request: sim_lib_agent_runner_core::ModelRequest) -> EvalRequest {
    EvalRequest {
        expr: Expr::from(model_request),
        result_shape: None,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::default(),
        mode: EvalMode::default(),
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn realize_eval_request(expr: Expr, required: &[Symbol]) -> EvalRequest {
    EvalRequest {
        expr,
        result_shape: None,
        required_capabilities: required.iter().map(capability_name).collect(),
        deadline: None,
        consistency: Consistency::default(),
        mode: EvalMode::default(),
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn author_outcome(
    task: &AuthorTask,
    checked_form: Option<Expr>,
    realized: Option<Expr>,
    attempts: Vec<RouteAttempt>,
    diagnostics: Vec<Diagnostic>,
) -> Result<AuthorOutcome> {
    let cassette = author_cassette(task, &attempts, &diagnostics)?;
    Ok(AuthorOutcome {
        checked_form,
        realized,
        attempts,
        diagnostics,
        cassette,
    })
}

fn author_cassette(
    task: &AuthorTask,
    attempts: &[RouteAttempt],
    diagnostics: &[Diagnostic],
) -> Result<DevCassette> {
    let mut events = Vec::new();
    if attempts.is_empty() {
        events.push(DevEvent::refusal(
            task.name.clone(),
            Expr::Map(vec![
                entry("target", Expr::String("none".to_owned())),
                entry(
                    "reason",
                    Expr::String(
                        first_diagnostic(diagnostics)
                            .unwrap_or("no route attempt")
                            .to_owned(),
                    ),
                ),
            ]),
        )?);
    } else {
        for attempt in attempts {
            let kind = if matches!(attempt.status, RouteAttemptStatus::Accepted) {
                CassetteKind::Validate
            } else {
                CassetteKind::Refusal
            };
            events.push(cassette_event(task, attempt, kind)?);
        }
    }
    DevCassette::from_events(
        Symbol::qualified("forge-author", task.name.as_qualified_str()),
        events,
    )
}

fn cassette_event(
    task: &AuthorTask,
    attempt: &RouteAttempt,
    kind: CassetteKind,
) -> Result<DevEvent> {
    let payload = Expr::Map(vec![
        entry("target", Expr::String(attempt.target.clone())),
        entry("status", Expr::Symbol(route_status_symbol(&attempt.status))),
        entry(
            "reason",
            attempt
                .reason
                .as_ref()
                .map(|reason| Expr::String(reason.clone()))
                .unwrap_or(Expr::Nil),
        ),
    ]);
    match kind {
        CassetteKind::Validate => DevEvent::validate(task.name.clone(), payload),
        CassetteKind::Refusal => DevEvent::refusal(task.name.clone(), payload),
    }
}

enum CassetteKind {
    Validate,
    Refusal,
}

fn target_skip_reason(target: &RouteTarget<'_>, ceiling: &[Symbol]) -> Option<String> {
    target
        .required_capabilities
        .iter()
        .find(|required| !capability_allowed(required, ceiling))
        .map(|required| format!("target requires capability {required} outside projection ceiling"))
}

fn outside_ceiling_reason(required: &[Symbol], ceiling: &[Symbol]) -> Option<String> {
    required
        .iter()
        .find(|required| !capability_allowed(required, ceiling))
        .map(|required| format!("form requires capability {required} outside projection ceiling"))
}

fn capability_allowed(required: &Symbol, ceiling: &[Symbol]) -> bool {
    let required = capability_name(required);
    ceiling
        .iter()
        .map(capability_name)
        .any(|allowed| allowed == required)
}

fn diminished_capabilities(current: &CapabilitySet, ceiling: &[Symbol]) -> CapabilitySet {
    let allowed = ceiling
        .iter()
        .map(capability_name)
        .fold(CapabilitySet::new(), CapabilitySet::grant);
    current.intersect(&allowed)
}

fn capability_name(symbol: &Symbol) -> CapabilityName {
    match symbol.namespace.as_deref() {
        Some("capability") => CapabilityName::new(symbol.name.as_ref()),
        _ => CapabilityName::new(symbol.to_string()),
    }
}

fn required_capabilities_for_form(expr: &Expr) -> Vec<Symbol> {
    let mut required = BTreeSet::new();
    collect_required_capabilities(expr, &mut required);
    required.into_iter().collect()
}

fn collect_required_capabilities(expr: &Expr, required: &mut BTreeSet<Symbol>) {
    match expr {
        Expr::Symbol(symbol) => {
            if symbol.namespace.as_deref() == Some("capability") {
                required.insert(symbol.clone());
            }
        }
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            for item in items {
                collect_required_capabilities(item, required);
            }
        }
        Expr::Map(entries) => {
            for (key, value) in entries {
                collect_required_capabilities(key, required);
                collect_required_capabilities(value, required);
            }
        }
        Expr::Call { operator, args } => {
            collect_required_capabilities(operator, required);
            for arg in args {
                collect_required_capabilities(arg, required);
            }
        }
        Expr::Infix {
            operator: _,
            left,
            right,
        } => {
            collect_required_capabilities(left, required);
            collect_required_capabilities(right, required);
        }
        Expr::Prefix { operator: _, arg } | Expr::Postfix { operator: _, arg } => {
            collect_required_capabilities(arg, required);
        }
        Expr::Quote { expr, .. } | Expr::Annotated { expr, .. } => {
            collect_required_capabilities(expr, required);
        }
        Expr::Extension { payload, .. } => collect_required_capabilities(payload, required),
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::Local(_)
        | Expr::String(_)
        | Expr::Bytes(_) => {}
    }
}

fn author_intent(task: &AuthorTask) -> CompiledIntent {
    CompiledIntent {
        name: task.name.clone(),
        verifiers: task.verifiers.clone(),
        status: IntentStatus::Verified,
        ..CompiledIntent::default()
    }
}

fn shape_diagnostics(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        "no diagnostics".to_owned()
    } else {
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn first_diagnostic(diagnostics: &[Diagnostic]) -> Option<&str> {
    diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.as_str())
}

fn route_status_symbol(status: &RouteAttemptStatus) -> Symbol {
    let name = match status {
        RouteAttemptStatus::Skipped => "skipped",
        RouteAttemptStatus::Failed => "failed",
        RouteAttemptStatus::Accepted => "accepted",
    };
    Symbol::qualified("forge-route", name)
}
