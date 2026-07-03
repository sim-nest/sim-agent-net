use super::market_cards::{card_context_tokens, card_cost, card_is_healthy, card_supports};
use super::market_policy::{MarketPolicy, key_expr};
use sim_kernel::{Consistency, Cx, Error, EvalMode, EvalRequest, Expr, Result};
use sim_lib_agent_runner_core::{ModelBid, ModelCard, ModelRequest, ModelResponse};
use std::time::Duration;

pub(super) fn realize_runner(
    cx: &mut Cx,
    runner: &sim_kernel::Value,
    request: &ModelRequest,
    deadline: Option<Duration>,
) -> Result<ModelResponse> {
    let fabric = runner
        .object()
        .as_eval_fabric()
        .ok_or_else(|| Error::Eval("runner/market candidate is not an EvalFabric".to_owned()))?;
    let reply = fabric.realize(
        cx,
        EvalRequest {
            expr: request.clone().into(),
            result_shape: None,
            required_capabilities: Vec::new(),
            deadline,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        },
    )?;
    ModelResponse::try_from(reply.value.object().as_expr(cx)?)
}

pub(super) fn verification_request(
    request: &ModelRequest,
    response: &ModelResponse,
) -> ModelRequest {
    ModelRequest::new(
        Expr::Map(vec![
            key_expr("verify-market-response", Expr::Bool(true)),
            key_expr("request", request.clone().into()),
            key_expr("response", response.clone().into()),
        ]),
        Vec::new(),
    )
}

pub(super) fn debate_judge_request(
    request: &ModelRequest,
    answers: &[ModelResponse],
) -> ModelRequest {
    ModelRequest {
        task: Expr::Map(vec![
            key_expr("debate", Expr::Bool(true)),
            key_expr("task", request.task.clone()),
            key_expr(
                "answers",
                Expr::List(answers.iter().cloned().map(Expr::from).collect()),
            ),
        ]),
        messages: request.messages.clone(),
        extra: request.extra.clone(),
    }
}

pub(super) fn candidate_accepted(policy: &MarketPolicy, card: &ModelCard, bid: &ModelBid) -> bool {
    if !bid.available || !card_is_healthy(card) {
        return false;
    }
    if policy
        .max_cost_usd
        .is_some_and(|max| card_cost(card).is_some_and(|cost| cost > max))
    {
        return false;
    }
    if policy
        .min_context_tokens
        .is_some_and(|min| card_context_tokens(card).is_none_or(|tokens| tokens < min))
    {
        return false;
    }
    policy
        .requires
        .iter()
        .all(|requirement| card_supports(card, requirement))
}

pub(super) fn candidate_score(policy: &MarketPolicy, card: &ModelCard, bid: &ModelBid) -> f64 {
    let cost = card_cost(card).unwrap_or(0.0);
    let base = bid.score.unwrap_or(cost);
    match policy.prefer.name.as_ref() {
        "auction" | "escalate" => cost,
        "local-first" if card.locality.name.as_ref() == "local" => base,
        "local-first" => base + 1_000.0,
        "carbon-aware" if card.locality.name.as_ref() == "local" => base,
        "carbon-aware" => base + 100.0,
        _ => base,
    }
}

pub(super) fn policy_deadline(policy: &MarketPolicy) -> Result<Option<Duration>> {
    policy
        .deadline
        .as_ref()
        .map(|deadline| sim_lib_server::parse_duration(&Expr::String(deadline.clone())))
        .transpose()
}

pub(super) fn deadline_for(policy: &MarketPolicy, card: &ModelCard) -> Result<Option<Duration>> {
    let policy = policy_deadline(policy)?;
    let runner = card
        .extra
        .iter()
        .find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == "timeout" => Some(value),
            _ => None,
        })
        .map(sim_lib_server::parse_duration)
        .transpose()?;
    Ok(match (policy, runner) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    })
}
