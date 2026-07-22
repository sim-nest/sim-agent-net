use std::sync::Arc;

use sim_codec_chat::{text_part, validate_chat_transcript};
use sim_kernel::{Args, Callable, Error, Expr, Symbol};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};

use crate::{
    OpenAiGatewayFunction, OpenAiPlanCache, OpenAiRunnerRegistry, PlanLimits, check_plan,
    check_plan_with_limits, eval_plan, eval_plan_report, eval_plan_report_with_cache_and_runners,
    explain_plan, install_openai_gateway_lib, openai_gateway_plan_capability,
    openai_gateway_plan_remote_capability, parse_plan, plan_combinators_expr, plan_parse_symbol,
    provider_prefixes, resolve_atom_address,
};

#[test]
fn plan_parse_atom_yields_plan_atom() {
    let plan = parse_plan("openai/gpt-4o-mini").unwrap();

    assert_eq!(plan_head(&plan), "plan/atom");
    assert_eq!(plan_arg_string(&plan, 0), "openai/gpt-4o-mini");
}

#[test]
fn plan_parse_race_yields_two_atom_children() {
    let plan = parse_plan("race(a, b)").unwrap();

    assert_eq!(plan_head(&plan), "plan/race");
    let args = plan_args(&plan);
    assert_eq!(args.len(), 2);
    assert_eq!(plan_head(&args[0]), "plan/atom");
    assert_eq!(plan_arg_string(&args[0], 0), "a");
    assert_eq!(plan_head(&args[1]), "plan/atom");
    assert_eq!(plan_arg_string(&args[1], 0), "b");
}

#[test]
fn plan_check_rejects_unknown_combinator_and_over_deep_nesting() {
    let unknown = parse_plan("unknown(a)").unwrap();
    let err = check_plan(&unknown).unwrap_err();
    assert!(format!("{err}").contains("unknown plan combinator"));

    let over_deep = parse_plan("race(race(race(race(a))))").unwrap();
    let err = check_plan(&over_deep).unwrap_err();
    assert!(format!("{err}").contains("maximum depth"));

    let over_fan_out = parse_plan("race(a, b, c)").unwrap();
    let err = check_plan_with_limits(
        &over_fan_out,
        PlanLimits {
            max_depth: 4,
            max_fan_out: 2,
        },
    )
    .unwrap_err();
    assert!(format!("{err}").contains("fan-out"));
}

#[test]
fn plan_explain_returns_readable_tree() {
    let plan = parse_plan("race(a, b)").unwrap();
    let explanation = explain_plan(&plan).unwrap();

    assert!(explanation.contains("plan/race"));
    assert!(explanation.contains("plan/atom a"));
    assert!(explanation.contains("plan/atom b"));
}

#[test]
fn fixture_echo_atom_eval_returns_model_response() {
    let mut cx = super::cx();
    let plan = parse_plan("fixture/echo").unwrap();
    let request = model_request("hello fixture");
    let response = eval_plan(&mut cx, &plan, &request).unwrap();

    validate_chat_transcript(&response).unwrap();
    let response = ModelResponse::try_from(response).unwrap();
    assert_eq!(response.runner, Symbol::new("fixture/echo"));
    assert_eq!(response.model, "fixture/echo");
    assert!(format!("{:?}", response.content).contains("hello fixture"));
}

#[test]
fn race_returns_fast_fixture_and_records_cancelled_branch() {
    let mut cx = plan_cx();
    let plan = parse_plan("race(fixture/echo, fixture/slow-echo)").unwrap();
    let request = model_request("race me");

    let report = eval_plan_report(&mut cx, &plan, &request).unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();

    assert_eq!(response.model, "fixture/echo");
    assert!(report_text(&response).contains("race me"));
    assert!(has_branch_end(&report.events, "winner"));
    assert!(has_branch_end(&report.events, "cancelled"));
}

#[test]
fn plan_combinators_require_plan_capability() {
    let mut cx = super::cx();
    let plan = parse_plan("race(fixture/echo, fixture/slow-echo)").unwrap();
    let request = model_request("missing capability");

    let err = eval_plan_report(&mut cx, &plan, &request).unwrap_err();

    assert!(format!("{err}").contains("openai-gateway.plan"));
}

#[test]
fn fallback_chain_budget_local_remote_and_trace_execute_with_fixtures() {
    let request = model_request("compose me");

    let mut cx = plan_cx();
    let fallback = parse_plan("fallback(fixture/fail, fixture/echo)").unwrap();
    let response = eval_plan_report(&mut cx, &fallback, &request).unwrap();
    assert!(format!("{:?}", response.response).contains("compose me"));
    assert!(has_branch_end(&response.events, "error"));
    assert!(has_branch_end(&response.events, "accepted"));

    for source in [
        "chain(fixture/echo, fixture/echo)",
        "budget(fixture/echo, max-tokens: 10)",
        "market(fixture/echo)",
        "local(fixture/echo)",
        "trace(fixture/echo)",
    ] {
        let mut cx = plan_cx();
        let plan = parse_plan(source).unwrap();
        let response = eval_plan_report(&mut cx, &plan, &request).unwrap();
        assert!(format!("{:?}", response.response).contains("compose me"));
        assert!(has_event_kind(&response.events, "branch-start"));
    }

    let mut cx = plan_cx();
    cx.grant(openai_gateway_plan_remote_capability());
    let remote = parse_plan("remote(fixture/echo)").unwrap();
    let response = eval_plan_report(&mut cx, &remote, &request).unwrap();
    assert!(format!("{:?}", response.response).contains("compose me"));
}

#[test]
fn verify_accepts_ok_checker_and_applies_on_fail() {
    let mut cx = plan_cx();
    let request = model_request("verify me");
    let accepted = parse_plan("verify(fixture/echo, fixture/always-ok)").unwrap();
    let accepted = eval_plan_report(&mut cx, &accepted, &request).unwrap();
    assert!(format!("{:?}", accepted.response).contains("verify me"));

    let rejected = parse_plan("verify(fixture/echo, fixture/always-fail)").unwrap();
    let err = eval_plan_report(&mut cx, &rejected, &request).unwrap_err();
    assert!(format!("{err}").contains("checker rejected"));

    let accept_on_fail =
        parse_plan("verify(fixture/echo, fixture/always-fail, on-fail: accept)").unwrap();
    let accepted = eval_plan_report(&mut cx, &accept_on_fail, &request).unwrap();
    assert!(format!("{:?}", accepted.response).contains("verify me"));
}

#[test]
fn debate_uses_judge_and_records_side_transcripts() {
    let mut cx = plan_cx();
    let request = model_request("debate me");
    let plan = parse_plan("debate(fixture/a, fixture/b, judge: fixture/judge)").unwrap();

    let report = eval_plan_report(&mut cx, &plan, &request).unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();
    let text = report_text(&response);

    assert!(text.contains("judged"));
    assert!(text.contains("fixture a transcript"));
    assert!(text.contains("fixture b transcript"));
    assert!(format!("{:?}", report.events).contains("debate/side"));
}

#[test]
fn remote_plan_is_rejected_under_local_only_privacy() {
    let mut cx = plan_cx();
    let plan = parse_plan("remote(fixture/echo)").unwrap();
    let request = model_request_with_privacy("stay local", "local-only");

    let err = eval_plan_report(&mut cx, &plan, &request).unwrap_err();

    assert!(format!("{err}").contains("local-only privacy"));
}

#[test]
fn plan_callables_are_registered_and_parse_plans() {
    let mut cx = super::cx();
    install_openai_gateway_lib(&mut cx).unwrap();

    assert!(cx.resolve_function(&plan_parse_symbol()).is_ok());
    let input = cx.factory().string("race(a, b)".to_owned()).unwrap();
    let parsed = OpenAiGatewayFunction::plan_parse()
        .call(&mut cx, Args::new(vec![input]))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();

    assert_eq!(plan_head(&parsed), "plan/race");
    assert!(matches!(plan_combinators_expr(), Expr::List(items) if !items.is_empty()));
}

#[test]
fn native_provider_prefixes_resolve_as_open_plan_data() {
    for prefix in ["openai", "anthropic", "ollama", "lm-studio", "lemonade"] {
        assert!(provider_prefixes().contains(&prefix));
        let address = format!("{prefix}/model");
        let descriptor = resolve_atom_address(&address).unwrap();
        assert_eq!(descriptor.head, prefix);
        assert!(descriptor.is_runner_backed());
    }
}

#[test]
fn native_provider_atom_dispatches_to_registered_runner() {
    let mut cx = plan_cx();
    let plan = parse_plan("anthropic/claude-haiku").unwrap();
    let request = model_request("native dispatch");
    let registry = runner_registry(vec![test_runner(
        "runner/anthropic",
        "anthropic/claude-haiku",
        "anthropic",
        "network",
        "claude ok",
    )]);
    let mut cache = OpenAiPlanCache::new();

    let report =
        eval_plan_report_with_cache_and_runners(&mut cx, &plan, &request, &mut cache, &registry)
            .unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();

    assert_eq!(response.runner, Symbol::new("runner/anthropic"));
    assert_eq!(response.model, "anthropic/claude-haiku");
    assert!(report_text(&response).contains("claude ok"));
}

#[test]
fn native_provider_fallback_accepts_later_runner() {
    let mut cx = plan_cx();
    let plan = parse_plan("fallback(anthropic/fail, lm-studio/local-default)").unwrap();
    let request = model_request("fallback dispatch");
    let registry = runner_registry(vec![
        failing_runner("runner/anthropic", "anthropic/fail", "anthropic", "network"),
        test_runner(
            "runner/lm-studio",
            "lm-studio/local-default",
            "lm-studio",
            "local",
            "lm studio ok",
        ),
    ]);
    let mut cache = OpenAiPlanCache::new();

    let report =
        eval_plan_report_with_cache_and_runners(&mut cx, &plan, &request, &mut cache, &registry)
            .unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();

    assert_eq!(response.model, "lm-studio/local-default");
    assert!(report_text(&response).contains("lm studio ok"));
    assert!(has_branch_end(&report.events, "error"));
    assert!(has_branch_end(&report.events, "accepted"));
}

#[test]
fn native_provider_race_records_winner_and_cancelled_runner() {
    let mut cx = plan_cx();
    let plan = parse_plan("race(ollama/qwen, lemonade/qwen)").unwrap();
    let request = model_request("race providers");
    let registry = runner_registry(vec![
        test_runner(
            "runner/ollama",
            "ollama/qwen",
            "ollama",
            "local",
            "ollama ok",
        ),
        test_runner(
            "runner/lemonade",
            "lemonade/qwen",
            "lemonade",
            "local",
            "lemonade ok",
        ),
    ]);
    let mut cache = OpenAiPlanCache::new();

    let report =
        eval_plan_report_with_cache_and_runners(&mut cx, &plan, &request, &mut cache, &registry)
            .unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();

    assert_eq!(response.model, "ollama/qwen");
    assert!(has_branch_end(&report.events, "winner"));
    assert!(has_branch_end(&report.events, "cancelled"));
}

#[test]
fn native_provider_local_only_uses_runner_card_locality() {
    let request = model_request_with_privacy("stay local", "local-only");
    let registry = runner_registry(vec![
        test_runner(
            "runner/lm-studio",
            "lm-studio/local-default",
            "lm-studio",
            "local",
            "local ok",
        ),
        test_runner(
            "runner/openai",
            "openai/hosted",
            "openai",
            "network",
            "hosted ok",
        ),
    ]);

    let mut cx = plan_cx();
    let mut cache = OpenAiPlanCache::new();
    let accepted = parse_plan("lm-studio/local-default").unwrap();
    let accepted = eval_plan_report_with_cache_and_runners(
        &mut cx, &accepted, &request, &mut cache, &registry,
    )
    .unwrap();
    let accepted = ModelResponse::try_from(accepted.response).unwrap();
    assert_eq!(accepted.model, "lm-studio/local-default");

    let mut cx = plan_cx();
    let mut cache = OpenAiPlanCache::new();
    let rejected = parse_plan("openai/hosted").unwrap();
    let err = eval_plan_report_with_cache_and_runners(
        &mut cx, &rejected, &request, &mut cache, &registry,
    )
    .unwrap_err();
    assert!(format!("{err}").contains("local-only privacy"));
}

#[test]
fn native_provider_missing_runner_reports_model_not_found() {
    let mut cx = plan_cx();
    let plan = parse_plan("anthropic/missing").unwrap();
    let request = model_request("missing provider");
    let registry = OpenAiRunnerRegistry::new();
    let mut cache = OpenAiPlanCache::new();

    let err =
        eval_plan_report_with_cache_and_runners(&mut cx, &plan, &request, &mut cache, &registry)
            .unwrap_err();

    assert!(format!("{err}").contains("model_not_found: anthropic/missing"));
}

fn plan_cx() -> sim_kernel::Cx {
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    cx
}

fn model_request(text: &str) -> Expr {
    model_request_entries(text, Vec::new())
}

fn model_request_with_privacy(text: &str, privacy: &str) -> Expr {
    model_request_entries(
        text,
        vec![(
            Expr::Symbol(Symbol::new("privacy")),
            Expr::String(privacy.to_owned()),
        )],
    )
}

fn model_request_entries(text: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(text.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("role")),
                    Expr::Symbol(Symbol::new("user")),
                ),
                (
                    Expr::Symbol(Symbol::new("content")),
                    Expr::List(vec![Expr::Map(vec![
                        (
                            Expr::Symbol(Symbol::new("type")),
                            Expr::Symbol(Symbol::new("text")),
                        ),
                        (
                            Expr::Symbol(Symbol::new("text")),
                            Expr::String(text.to_owned()),
                        ),
                    ])]),
                ),
            ])]),
        ),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn report_text(response: &ModelResponse) -> String {
    format!("{:?}", response.content)
}

fn has_event_kind(events: &[crate::PlanEvalEvent], kind: &str) -> bool {
    events.iter().any(|event| event.kind.name.as_ref() == kind)
}

fn has_branch_end(events: &[crate::PlanEvalEvent], status: &str) -> bool {
    events.iter().any(|event| {
        event.kind.name.as_ref() == "branch-end" && format!("{:?}", event.payload).contains(status)
    })
}

fn runner_registry(runners: Vec<TestRunner>) -> OpenAiRunnerRegistry {
    let mut registry = OpenAiRunnerRegistry::new();
    for runner in runners {
        registry.register(runner.model, Arc::new(runner));
    }
    registry
}

fn test_runner(
    runner: &'static str,
    model: &'static str,
    provider: &'static str,
    locality: &'static str,
    text: &'static str,
) -> TestRunner {
    TestRunner {
        runner,
        model,
        provider,
        locality,
        text,
        fail: false,
    }
}

fn failing_runner(
    runner: &'static str,
    model: &'static str,
    provider: &'static str,
    locality: &'static str,
) -> TestRunner {
    TestRunner {
        runner,
        model,
        provider,
        locality,
        text: "failed",
        fail: true,
    }
}

#[derive(Clone)]
struct TestRunner {
    runner: &'static str,
    model: &'static str,
    provider: &'static str,
    locality: &'static str,
    text: &'static str,
    fail: bool,
}

impl ModelRunner for TestRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            Symbol::new(self.runner),
            self.model,
            Symbol::new(self.provider),
            Symbol::new(self.locality),
        )
    }

    fn infer(
        &self,
        _cx: &mut sim_kernel::Cx,
        _request: ModelRequest,
    ) -> sim_kernel::Result<ModelResponse> {
        if self.fail {
            return Err(Error::Eval(format!("{} failed", self.model)));
        }
        Ok(ModelResponse::new(
            Symbol::new(self.runner),
            self.model,
            vec![text_part(self.text)],
            Symbol::new("stop"),
        ))
    }
}

fn plan_head(plan: &Expr) -> &str {
    let Expr::List(items) = plan else {
        panic!("plan must be a list");
    };
    let Some(Expr::Symbol(symbol)) = items.first() else {
        panic!("plan must have a symbol head");
    };
    symbol.name.as_ref()
}

fn plan_args(plan: &Expr) -> &[Expr] {
    let Expr::List(items) = plan else {
        panic!("plan must be a list");
    };
    &items[1..]
}

fn plan_arg_string(plan: &Expr, index: usize) -> &str {
    let Expr::String(value) = &plan_args(plan)[index] else {
        panic!("plan arg must be a string");
    };
    value
}
