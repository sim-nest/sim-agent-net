use super::market_cards::{runner_bid, runner_card};
use super::market_decision::{
    MarketCandidate, MarketDecision, is_error_response, request_allows_branching,
    response_failure_reason, response_should_escalate, verifier_accepts,
};
use super::market_execution_support::{
    candidate_accepted, candidate_score, deadline_for, debate_judge_request, policy_deadline,
    realize_runner, verification_request,
};
use super::market_policy::MarketPolicy;
use crate::model_privacy::PrivacyPolicy;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};

pub(crate) fn execute_market(
    cx: &mut Cx,
    runners: &[Value],
    policy: MarketPolicy,
    request: ModelRequest,
    privacy: PrivacyPolicy,
) -> Result<ModelResponse> {
    let mut executor = MarketExecutor { runners, policy };
    let mut decision = MarketDecision::new(executor.policy.clone(), privacy);
    let mut response = match executor.execution_mode() {
        "race" => executor.execute_race(cx, &request, &mut decision)?,
        "speculate" => executor.execute_speculate(cx, &request, &mut decision)?,
        "debate" => executor.execute_debate(cx, &request, &mut decision)?,
        "escalate" => executor.execute_escalate(cx, &request, &mut decision)?,
        _ => executor.execute_select(cx, &request, &mut decision)?,
    };
    decision.selected_response = Some(Expr::from(response.clone()));
    response.extra.push(super::market_policy::key_expr(
        "market-decision",
        decision.to_expr(),
    ));
    Ok(response)
}

struct MarketExecutor<'a> {
    runners: &'a [Value],
    policy: MarketPolicy,
}

impl MarketExecutor<'_> {
    fn execution_mode(&self) -> &str {
        let mode = self
            .policy
            .execution
            .as_ref()
            .unwrap_or(&self.policy.prefer)
            .name
            .as_ref();
        match mode {
            "race" | "speculate" | "debate" | "escalate" => mode,
            _ => "select",
        }
    }

    fn execute_select(
        &mut self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        let candidates = self.candidates(cx, request, decision)?;
        let selected = candidates
            .iter()
            .min_by(|left, right| left.score.total_cmp(&right.score))
            .ok_or_else(|| Error::Eval("runner/market found no valid runner".to_owned()))?;
        let mut response = match self.realize_candidate(cx, selected, request, "select", decision) {
            Ok(response) => response,
            Err(error) => {
                decision.failure(selected.symbol(), error.to_string());
                self.realize_fallback(
                    cx,
                    request,
                    decision,
                    &candidates,
                    Some(&selected.card.runner),
                )?
                .ok_or(error)?
            }
        };
        if super::market_cards::is_retryable_error(&response)
            && let Some(fallback) = self.realize_fallback(
                cx,
                request,
                decision,
                &candidates,
                Some(&selected.card.runner),
            )?
        {
            response = fallback;
        }
        self.verify_selected(cx, request, &mut response, decision)?;
        Ok(response)
    }

    fn execute_race(
        &mut self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        if !request_allows_branching(request) {
            decision.failure(
                Symbol::qualified("runner", "market"),
                "non-idempotent tool continuation used select execution".to_owned(),
            );
            return self.execute_select(cx, request, decision);
        }
        let mut candidates = self.candidates(cx, request, decision)?;
        candidates.sort_by(|left, right| {
            left.latency_ms()
                .cmp(&right.latency_ms())
                .then_with(|| left.score.total_cmp(&right.score))
        });
        let mut last_error = None;
        for index in 0..candidates.len() {
            let candidate = &candidates[index];
            match self.realize_candidate(cx, candidate, request, "race", decision) {
                Ok(response) if !response_should_escalate(&response) => {
                    for cancelled in candidates.iter().skip(index + 1) {
                        decision.cancelled(cancelled.symbol(), "race winner selected");
                    }
                    return Ok(response);
                }
                Ok(response) => {
                    decision.failure(
                        candidate.symbol(),
                        response_failure_reason(&response)
                            .unwrap_or_else(|| "response rejected".to_owned()),
                    );
                    last_error = Some(Error::Eval("race candidate response rejected".to_owned()));
                }
                Err(error) => {
                    decision.failure(candidate.symbol(), error.to_string());
                    last_error = Some(error);
                }
            }
        }
        self.realize_fallback(cx, request, decision, &candidates, None)?
            .ok_or_else(|| {
                last_error.unwrap_or_else(|| {
                    Error::Eval("runner/market race found no valid runner".to_owned())
                })
            })
    }

    fn execute_speculate(
        &mut self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        if !request_allows_branching(request) {
            decision.failure(
                Symbol::qualified("runner", "market"),
                "non-idempotent tool continuation used select execution".to_owned(),
            );
            return self.execute_select(cx, request, decision);
        }
        let mut candidates = self.branch_candidates(cx, request, decision)?;
        candidates.sort_by(|left, right| left.score.total_cmp(&right.score));
        let cheap = candidates.first().ok_or_else(|| {
            Error::Eval("runner/market speculate found no valid runner".to_owned())
        })?;
        let expensive = candidates.get(1).unwrap_or(cheap);
        let expensive_response = if expensive.card.runner == cheap.card.runner {
            None
        } else {
            Some(self.realize_candidate(cx, expensive, request, "speculate-expensive", decision))
        };
        let cheap_response =
            match self.realize_candidate(cx, cheap, request, "speculate-cheap", decision) {
                Ok(response) if !response_should_escalate(&response) => response,
                Ok(response) => {
                    decision.failure(
                        cheap.symbol(),
                        response_failure_reason(&response)
                            .unwrap_or_else(|| "cheap branch rejected".to_owned()),
                    );
                    return expensive_response
                        .transpose()?
                        .ok_or_else(|| Error::Eval("speculate cheap branch rejected".to_owned()));
                }
                Err(error) => {
                    decision.failure(cheap.symbol(), error.to_string());
                    return expensive_response.transpose()?.ok_or(error);
                }
            };
        if self.verify_candidate(cx, request, &cheap_response, decision)? {
            decision.selected = Some(cheap.card.runner.clone());
            if expensive.card.runner != cheap.card.runner {
                decision.cancelled(expensive.symbol(), "cheap branch verified");
            }
            Ok(cheap_response)
        } else {
            decision.failure(
                cheap.symbol(),
                "cheap branch failed verification".to_owned(),
            );
            expensive_response.transpose()?.ok_or_else(|| {
                Error::Eval("speculate verifier rejected the only branch".to_owned())
            })
        }
    }

    fn execute_debate(
        &mut self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        if !request_allows_branching(request) {
            decision.failure(
                Symbol::qualified("runner", "market"),
                "non-idempotent tool continuation used select execution".to_owned(),
            );
            return self.execute_select(cx, request, decision);
        }
        let judge = self.judge_symbol().ok_or_else(|| {
            Error::Eval("runner/market debate requires :judge or :verify-with".to_owned())
        })?;
        let candidates = self.candidates(cx, request, decision)?;
        let judge_runner = candidates
            .iter()
            .find(|candidate| candidate.card.runner == judge)
            .map(|candidate| candidate.runner.clone())
            .ok_or_else(|| Error::Eval(format!("debate judge {judge} not accepted")))?;
        let mut answers = Vec::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.card.runner != judge)
        {
            match self.realize_candidate(cx, candidate, request, "debate", decision) {
                Ok(response) if !is_error_response(&response) => answers.push(response),
                Ok(response) => decision.failure(
                    candidate.symbol(),
                    response_failure_reason(&response)
                        .unwrap_or_else(|| "debate answer rejected".to_owned()),
                ),
                Err(error) => decision.failure(candidate.symbol(), error.to_string()),
            }
        }
        if answers.is_empty() {
            return Err(Error::Eval(
                "runner/market debate produced no candidate answers".to_owned(),
            ));
        }
        let judge_request = debate_judge_request(request, &answers);
        let mut response = realize_runner(
            cx,
            &judge_runner,
            &judge_request,
            policy_deadline(&self.policy)?,
        )?;
        response.extra.push(super::market_policy::key_expr(
            "debate-answers",
            Expr::List(answers.iter().cloned().map(Expr::from).collect()),
        ));
        response.extra.push(super::market_policy::key_expr(
            "debate-judge",
            Expr::Symbol(judge.clone()),
        ));
        decision.selected = Some(judge);
        Ok(response)
    }

    fn execute_escalate(
        &mut self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        let mut candidates = self.candidates(cx, request, decision)?;
        candidates.sort_by(|left, right| left.score.total_cmp(&right.score));
        let mut last_response = None;
        let mut last_error = None;
        for candidate in &candidates {
            if self.policy.fallback.as_ref() == Some(&candidate.card.runner) {
                decision.fallback_used = Some(candidate.card.runner.clone());
            }
            match self.realize_candidate(cx, candidate, request, "escalate", decision) {
                Ok(response) if !response_should_escalate(&response) => return Ok(response),
                Ok(response) => {
                    decision.failure(
                        candidate.symbol(),
                        response_failure_reason(&response)
                            .unwrap_or_else(|| "escalation condition matched".to_owned()),
                    );
                    last_response = Some(response);
                }
                Err(error) => {
                    decision.failure(candidate.symbol(), error.to_string());
                    last_error = Some(error);
                }
            }
        }
        if let Some(response) = self.realize_fallback(cx, request, decision, &candidates, None)? {
            return Ok(response);
        }
        if let Some(response) = last_response {
            return Ok(response);
        }
        Err(last_error.unwrap_or_else(|| {
            Error::Eval("runner/market escalate found no valid runner".to_owned())
        }))
    }

    fn candidates(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<Vec<MarketCandidate>> {
        let mut candidates = Vec::new();
        for runner in self.runners {
            let card = runner_card(cx, runner)?;
            if let Some(reason) = decision.privacy.card_denial(&card) {
                decision.privacy_rejected(&card, reason);
                continue;
            }
            let bid = runner_bid(runner, request, &card);
            let score = candidate_score(&self.policy, &card, &bid);
            let accepted = candidate_accepted(&self.policy, &card, &bid);
            decision.bid(&card, &bid, score, accepted);
            if accepted {
                let candidate = MarketCandidate {
                    runner: runner.clone(),
                    card,
                    bid,
                    score,
                };
                decision.candidate(&candidate);
                candidates.push(candidate);
            }
        }
        Ok(candidates)
    }

    fn branch_candidates(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
    ) -> Result<Vec<MarketCandidate>> {
        let verify = self.verify_symbol();
        let judge = self.judge_symbol();
        Ok(self
            .candidates(cx, request, decision)?
            .into_iter()
            .filter(|candidate| {
                verify.as_ref() != Some(&candidate.card.runner)
                    && judge.as_ref() != Some(&candidate.card.runner)
            })
            .collect())
    }

    fn realize_candidate(
        &self,
        cx: &mut Cx,
        candidate: &MarketCandidate,
        request: &ModelRequest,
        phase: &str,
        decision: &mut MarketDecision,
    ) -> Result<ModelResponse> {
        decision.run(candidate.symbol(), phase);
        let response = realize_runner(
            cx,
            &candidate.runner,
            request,
            deadline_for(&self.policy, &candidate.card)?,
        )?;
        decision.response(candidate.symbol(), phase, &response);
        if !response_should_escalate(&response) {
            decision.selected = Some(candidate.card.runner.clone());
        }
        Ok(response)
    }

    fn realize_fallback(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
        candidates: &[MarketCandidate],
        skip: Option<&Symbol>,
    ) -> Result<Option<ModelResponse>> {
        let Some(fallback) = &self.policy.fallback else {
            return Ok(None);
        };
        if skip.is_some_and(|selected| selected == fallback) {
            return Ok(None);
        }
        let Some(candidate) = candidates
            .iter()
            .find(|candidate| &candidate.card.runner == fallback)
        else {
            decision.failure(fallback.clone(), "fallback runner not accepted".to_owned());
            return Ok(None);
        };
        decision.fallback_used = Some(fallback.clone());
        self.realize_candidate(cx, candidate, request, "fallback", decision)
            .map(Some)
    }

    fn verify_selected(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        response: &mut ModelResponse,
        decision: &mut MarketDecision,
    ) -> Result<()> {
        let Some(verifier) = &self.policy.verify_with else {
            return Ok(());
        };
        let Some(verify_runner) =
            self.accepted_runner_by_symbol(cx, request, decision, verifier)?
        else {
            decision.failure(verifier.clone(), "verifier runner not found".to_owned());
            return Ok(());
        };
        match realize_runner(
            cx,
            &verify_runner,
            &verification_request(request, response),
            policy_deadline(&self.policy)?,
        ) {
            Ok(check) => {
                decision.verified_by = Some(check.runner.clone());
                response.extra.push(super::market_policy::key_expr(
                    "market-verification",
                    Expr::from(check),
                ));
            }
            Err(error) => decision.failure(verifier.clone(), error.to_string()),
        }
        Ok(())
    }

    fn verify_candidate(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        response: &ModelResponse,
        decision: &mut MarketDecision,
    ) -> Result<bool> {
        let Some(verifier) = self.verify_symbol() else {
            return Ok(!response_should_escalate(response));
        };
        let Some(verify_runner) =
            self.accepted_runner_by_symbol(cx, request, decision, &verifier)?
        else {
            decision.failure(verifier.clone(), "verifier runner not found".to_owned());
            return Ok(false);
        };
        let check = realize_runner(
            cx,
            &verify_runner,
            &verification_request(request, response),
            policy_deadline(&self.policy)?,
        )?;
        decision.verified_by = Some(check.runner.clone());
        decision.response(verifier, "verify", &check);
        Ok(verifier_accepts(&check))
    }

    fn verify_symbol(&self) -> Option<Symbol> {
        self.policy
            .verify_with
            .clone()
            .or_else(|| self.policy.judge.clone())
    }

    fn judge_symbol(&self) -> Option<Symbol> {
        self.policy
            .judge
            .clone()
            .or_else(|| self.policy.verify_with.clone())
    }

    fn accepted_runner_by_symbol(
        &self,
        cx: &mut Cx,
        request: &ModelRequest,
        decision: &mut MarketDecision,
        symbol: &Symbol,
    ) -> Result<Option<Value>> {
        Ok(self
            .candidates(cx, request, decision)?
            .into_iter()
            .find(|candidate| &candidate.card.runner == symbol)
            .map(|candidate| candidate.runner))
    }
}
