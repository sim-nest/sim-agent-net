use std::mem;

use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};

use crate::{
    capabilities::{openai_gateway_plan_capability, openai_gateway_plan_remote_capability},
    plan::{
        address::resolve_atom_address,
        eval_context::{EvalContext, PlanPrivacy},
        eval_helpers::{
            branch_id, child_args, field, is_slow_fixture, keyword_atom, keyword_value,
            verifier_accepts,
        },
        fixtures::{
            fixture_echo_response, fixture_static_response, fixture_tool_call_response,
            request_text, response_summary, response_text,
        },
        shape::{check_plan, plan_parts},
    },
    runtime::{
        OpenAiFederation, OpenAiPlanCache, OpenAiRunnerRegistry, PlanCacheKey, PlanCacheMode,
        PlanCacheWriteTarget,
    },
};

/// Result of evaluating a plan: the final response expression and the trace of
/// branch events recorded during evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanEvalReport {
    /// Final model response, encoded as an [`Expr`].
    pub response: Expr,
    /// Ordered branch lifecycle events emitted while evaluating the plan.
    pub events: Vec<PlanEvalEvent>,
}

/// A single event recorded during plan evaluation, such as a branch start or end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEvalEvent {
    /// Event kind symbol (for example `branch-start` or `branch-end`).
    pub kind: Symbol,
    /// Structured event payload describing the branch and its outcome.
    pub payload: Expr,
}

/// Evaluates a plan against a request and returns only the final response expression.
pub fn eval_plan(cx: &mut Cx, plan: &Expr, request: &Expr) -> Result<Expr> {
    eval_plan_report(cx, plan, request).map(|report| report.response)
}

/// Evaluates a plan and returns the response together with its event trace.
pub fn eval_plan_report(cx: &mut Cx, plan: &Expr, request: &Expr) -> Result<PlanEvalReport> {
    eval_plan_report_inner(cx, plan, request, None, None, None)
}

/// Evaluates a plan with a plan cache available for `plan/cache` combinators.
pub fn eval_plan_report_with_cache(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    cache: &mut OpenAiPlanCache,
) -> Result<PlanEvalReport> {
    eval_plan_report_inner(cx, plan, request, Some(cache), None, None)
}

/// Evaluates a plan with both a plan cache and a runner registry for local inference.
pub fn eval_plan_report_with_cache_and_runners(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    cache: &mut OpenAiPlanCache,
    runners: &OpenAiRunnerRegistry,
) -> Result<PlanEvalReport> {
    eval_plan_report_inner(cx, plan, request, Some(cache), Some(runners), None)
}

/// Evaluates a plan with a federation handle for `gateway/*` remote atoms.
pub fn eval_plan_report_with_federation(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    federation: &OpenAiFederation,
) -> Result<PlanEvalReport> {
    eval_plan_report_inner(cx, plan, request, None, None, Some(federation))
}

/// Evaluates a plan with a plan cache, runner registry, and federation handle
/// all available.
pub fn eval_plan_report_with_cache_runners_and_federation(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    cache: &mut OpenAiPlanCache,
    runners: &OpenAiRunnerRegistry,
    federation: &OpenAiFederation,
) -> Result<PlanEvalReport> {
    eval_plan_report_inner(
        cx,
        plan,
        request,
        Some(cache),
        Some(runners),
        Some(federation),
    )
}

fn eval_plan_report_inner(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    cache: Option<&mut OpenAiPlanCache>,
    runners: Option<&OpenAiRunnerRegistry>,
    federation: Option<&OpenAiFederation>,
) -> Result<PlanEvalReport> {
    check_plan(plan)?;
    let (name, _) = plan_parts(plan)?;
    if name != "atom" {
        cx.require(&openai_gateway_plan_capability())?;
    }
    let request_model = ModelRequest::try_from(request.clone())?;
    let context = EvalContext::from_request(request);
    let mut evaluator = PlanEvaluator {
        cx,
        request: request_model,
        cache,
        runners,
        federation,
        events: Vec::new(),
        next_branch: 0,
    };
    let response = evaluator.eval(plan, context)?;
    Ok(PlanEvalReport {
        response: Expr::from(response),
        events: evaluator.events,
    })
}

struct PlanEvaluator<'a> {
    cx: &'a mut Cx,
    request: ModelRequest,
    cache: Option<&'a mut OpenAiPlanCache>,
    runners: Option<&'a OpenAiRunnerRegistry>,
    federation: Option<&'a OpenAiFederation>,
    events: Vec<PlanEvalEvent>,
    next_branch: u64,
}

impl PlanEvaluator<'_> {
    fn eval(&mut self, plan: &Expr, context: EvalContext) -> Result<ModelResponse> {
        let (name, args) = plan_parts(plan)?;
        if name == "atom" {
            let [Expr::String(address)] = args else {
                return Err(Error::Eval("plan/atom expects one address".to_owned()));
            };
            return self.eval_atom(address, context);
        }
        let children = child_args(args);
        match name {
            "race" => self.eval_race(&children, context),
            "fallback" => self.eval_fallback(&children, context),
            "chain" => self.eval_chain(&children, context),
            "budget" => self.eval_wrapped(name, children[0], context.budgeted(args)),
            "market" => self.eval_wrapped(name, children[0], context),
            "local" => self.eval_wrapped(name, children[0], context.local()),
            "remote" => self.eval_remote(children[0], context),
            "trace" => self.eval_wrapped(name, children[0], context.trace()),
            "verify" => self.eval_verify(args, &children, context),
            "debate" => self.eval_debate(args, &children, context),
            "cache" => self.eval_cache(args, children[0], context),
            _ => Err(Error::Eval(format!("unknown plan combinator {name}"))),
        }
    }

    fn eval_atom(&mut self, address: &str, context: EvalContext) -> Result<ModelResponse> {
        let descriptor = resolve_atom_address(address)?;
        if descriptor.is_gateway() {
            if context.privacy == PlanPrivacy::LocalOnly {
                return Err(Error::Eval(format!(
                    "local-only privacy rejects remote model {address}"
                )));
            }
            return self
                .federation
                .ok_or_else(|| Error::Eval(format!("model_not_found: {address}")))?
                .infer(
                    self.cx,
                    address,
                    &self.request,
                    &context.federation_policy(),
                );
        }
        if descriptor.is_runner_backed() {
            let runners = self
                .runners
                .ok_or_else(|| Error::Eval(format!("model_not_found: {address}")))?;
            let card = runners
                .card_for(address)
                .ok_or_else(|| Error::Eval(format!("model_not_found: {address}")))?;
            if context.privacy == PlanPrivacy::LocalOnly
                && !locality_is_allowed_for_local_only(&card.locality)
            {
                return Err(Error::Eval(format!(
                    "local-only privacy rejects remote model {address}"
                )));
            }
            return runners.infer(self.cx, address, self.request.clone());
        }
        match descriptor.address.as_str() {
            "fixture/echo" | "fixture/slow-echo" => {
                Ok(fixture_echo_response(address, &self.request))
            }
            "fixture/a" => Ok(fixture_static_response(address, "fixture a transcript")),
            "fixture/b" => Ok(fixture_static_response(address, "fixture b transcript")),
            "fixture/judge" => Ok(fixture_static_response(
                address,
                &format!("judged: {}", request_text(&self.request)),
            )),
            "fixture/tool-call" => Ok(fixture_tool_call_response(address, &self.request, false)),
            "fixture/repeat-tool-call" => {
                Ok(fixture_tool_call_response(address, &self.request, true))
            }
            "fixture/always-ok" => Ok(fixture_static_response(address, "ok")),
            "fixture/always-fail" => Ok(fixture_static_response(address, "fail")),
            "fixture/fail" => Err(Error::Eval("fixture/fail failed".to_owned())),
            _ => Ok(fixture_static_response(
                address,
                &request_text(&self.request),
            )),
        }
    }

    fn eval_race(&mut self, children: &[&Expr], context: EvalContext) -> Result<ModelResponse> {
        let mut winner = None;
        let mut first_error = None;
        for child in children {
            let branch = self.branch_start("race", child);
            if children.len() > 1 && is_slow_fixture(child) {
                self.branch_end(
                    branch,
                    "cancelled",
                    Expr::String("cancelled by faster branch".to_owned()),
                );
                continue;
            }
            match self.eval(child, context.clone()) {
                Ok(response) if winner.is_none() => {
                    self.branch_end(branch, "winner", response_summary(&response));
                    winner = Some(response);
                }
                Ok(response) => {
                    self.branch_end(branch, "cancelled", response_summary(&response));
                }
                Err(err) => {
                    self.branch_end(branch, "error", Expr::String(err.to_string()));
                    first_error.get_or_insert(err);
                }
            }
        }
        winner.ok_or_else(|| {
            first_error.unwrap_or_else(|| Error::Eval("plan/race had no winning branch".to_owned()))
        })
    }

    fn eval_fallback(&mut self, children: &[&Expr], context: EvalContext) -> Result<ModelResponse> {
        let mut first_error = None;
        for child in children {
            let branch = self.branch_start("fallback", child);
            match self.eval(child, context.clone()) {
                Ok(response) => {
                    self.branch_end(branch, "accepted", response_summary(&response));
                    return Ok(response);
                }
                Err(err) => {
                    self.branch_end(branch, "error", Expr::String(err.to_string()));
                    first_error.get_or_insert(err);
                }
            }
        }
        Err(first_error.unwrap_or_else(|| Error::Eval("plan/fallback had no children".to_owned())))
    }

    fn eval_chain(&mut self, children: &[&Expr], context: EvalContext) -> Result<ModelResponse> {
        let saved = self.request.clone();
        let mut last = None;
        for child in children {
            let response = self.eval_child("chain", child, context.clone())?;
            self.request = ModelRequest::new(Expr::String(response_text(&response)), Vec::new());
            last = Some(response);
        }
        self.request = saved;
        last.ok_or_else(|| Error::Eval("plan/chain had no children".to_owned()))
    }

    fn eval_wrapped(
        &mut self,
        combinator: &str,
        child: &Expr,
        context: EvalContext,
    ) -> Result<ModelResponse> {
        self.eval_child(combinator, child, context)
    }

    fn eval_remote(&mut self, child: &Expr, context: EvalContext) -> Result<ModelResponse> {
        if context.privacy == PlanPrivacy::LocalOnly {
            return Err(Error::Eval(
                "local-only privacy rejects plan/remote".to_owned(),
            ));
        }
        self.cx.require(&openai_gateway_plan_remote_capability())?;
        self.eval_child("remote", child, context)
    }

    fn eval_cache(
        &mut self,
        args: &[Expr],
        child: &Expr,
        context: EvalContext,
    ) -> Result<ModelResponse> {
        let mode = keyword_value(args, "mode")
            .map(PlanCacheMode::from_expr)
            .transpose()?
            .unwrap_or_default();
        if mode == PlanCacheMode::Disabled {
            return self.eval_child("cache", child, context);
        }

        let request_expr = Expr::from(self.request.clone());
        let key = PlanCacheKey::for_request_plan(&request_expr, child)?;
        if mode.reads()
            && let Some(cached) = self
                .cache
                .as_ref()
                .and_then(|cache| cache.get(&key))
                .cloned()
        {
            self.events.push(PlanEvalEvent {
                kind: Symbol::new("cache-hit"),
                payload: key.to_expr(),
            });
            return ModelResponse::try_from(cached);
        }

        let response = self.eval_child("cache", child, context)?;
        if mode.writes()
            && let Some(cache) = &mut self.cache
        {
            cache.put(
                &mut *self.cx,
                PlanCacheWriteTarget::Memory,
                key,
                Expr::from(response.clone()),
            )?;
        }
        Ok(response)
    }

    fn eval_verify(
        &mut self,
        args: &[Expr],
        children: &[&Expr],
        context: EvalContext,
    ) -> Result<ModelResponse> {
        let generator = self.eval_child("verify/generator", children[0], context.clone())?;
        let verifier = self.eval_child("verify/checker", children[1], context.clone())?;
        if verifier_accepts(&verifier) {
            return Ok(generator);
        }
        match keyword_atom(args, "on-fail").unwrap_or("error") {
            "accept" => Ok(generator),
            "error" | "escalate" => Err(Error::Eval(
                "plan/verify checker rejected output".to_owned(),
            )),
            other => Err(Error::Eval(format!(
                "unsupported plan/verify on-fail behavior {other}"
            ))),
        }
    }

    fn eval_debate(
        &mut self,
        args: &[Expr],
        children: &[&Expr],
        context: EvalContext,
    ) -> Result<ModelResponse> {
        let mut transcripts = Vec::new();
        for child in children {
            let response = self.eval_child("debate/side", child, context.clone())?;
            transcripts.push(response_text(&response));
        }
        let judge_plan = keyword_value(args, "judge").cloned().unwrap_or_else(|| {
            Expr::List(vec![
                Expr::Symbol(Symbol::new("plan/atom")),
                Expr::String("fixture/judge".to_owned()),
            ])
        });
        let judge_input = transcripts.join(" | ");
        self.with_request(
            ModelRequest::new(Expr::String(judge_input), Vec::new()),
            |evaluator| evaluator.eval_child("debate/judge", &judge_plan, context),
        )
    }

    fn eval_child(
        &mut self,
        combinator: &str,
        child: &Expr,
        context: EvalContext,
    ) -> Result<ModelResponse> {
        let branch = self.branch_start(combinator, child);
        match self.eval(child, context) {
            Ok(response) => {
                self.branch_end(branch, "completed", response_summary(&response));
                Ok(response)
            }
            Err(err) => {
                self.branch_end(branch, "error", Expr::String(err.to_string()));
                Err(err)
            }
        }
    }

    fn with_request<T>(
        &mut self,
        request: ModelRequest,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let saved = mem::replace(&mut self.request, request);
        let result = f(self);
        self.request = saved;
        result
    }

    fn branch_start(&mut self, combinator: &str, child: &Expr) -> u64 {
        let branch = self.next_branch;
        self.next_branch += 1;
        self.events.push(PlanEvalEvent {
            kind: Symbol::new("branch-start"),
            payload: Expr::Map(vec![
                field("branch-id", branch_id(branch)),
                field(
                    "combinator",
                    Expr::Symbol(Symbol::new(combinator.to_owned())),
                ),
                field("plan", child.clone()),
            ]),
        });
        branch
    }

    fn branch_end(&mut self, branch: u64, status: &str, result: Expr) {
        self.events.push(PlanEvalEvent {
            kind: Symbol::new("branch-end"),
            payload: Expr::Map(vec![
                field("branch-id", branch_id(branch)),
                field("status", Expr::Symbol(Symbol::new(status.to_owned()))),
                field("result", result),
            ]),
        });
    }
}

fn locality_is_allowed_for_local_only(locality: &Symbol) -> bool {
    matches!(
        locality.name.as_ref(),
        "local" | "agent" | "agent-backed" | "fabric" | "in-process" | "process"
    )
}
