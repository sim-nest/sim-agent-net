//! Offline authoring benchmark for contract-native FORGE requests.

use std::sync::{Arc, Mutex};

use sim_kernel::{
    Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, NumberLiteral, Result, Shape, Symbol,
};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};
use sim_lib_stream_core::{DevCassette, DevEvent};
use sim_value::build::entry;

use crate::{
    AuthorTask, ContractProjectionCaps, RankedContractCard, RouteAttempt, RouteAttemptStatus,
    RoutePolicy, RouteTarget, ShapeQuery, estimate_prompt_tokens, project_contracts,
    run_author_task,
};

const CHEAP_COST: u64 = 1;
const ESCALATION_COST: u64 = 3;
const HIGH_TIER_COST: u64 = 10;

/// Contract-native authoring arms measured by [`run_author_bench`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorArm {
    /// A raw source prompt sent to one high-tier fake runner.
    SourcePayload,
    /// A contract-projection prompt without strict output grammar.
    ContractPayload,
    /// The same contract projection with strict output grammar metadata.
    ContractGrammar,
    /// A strict contract request routed through cheap-first deterministic fakes.
    Downshifted,
}

impl AuthorArm {
    /// Stable report label for this arm.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SourcePayload => "source-payload",
            Self::ContractPayload => "contract-payload",
            Self::ContractGrammar => "contract-grammar",
            Self::Downshifted => "downshifted",
        }
    }
}

/// One deterministic authoring case.
#[derive(Clone)]
pub struct AuthorCase {
    /// Stable case id.
    pub name: Symbol,
    /// Baseline source prompt used by [`AuthorArm::SourcePayload`].
    pub source_payload: String,
    /// Human goal used by contract-native authoring arms.
    pub goal: String,
    /// Ranked contract cards projected into the model request.
    pub contract_cards: Vec<RankedContractCard>,
    /// Codec used for terminal model output.
    pub target_codec: Symbol,
    /// Data expression naming the return shape in model-request metadata.
    pub return_shape_expr: Expr,
    /// Return shape checked after decoding and realization.
    pub return_shape: Arc<dyn Shape>,
    /// Deterministic form returned by accepting fake runners.
    pub expected_form: Expr,
}

impl AuthorCase {
    /// Builds one deterministic authoring case.
    pub fn new(
        name: Symbol,
        source_payload: impl Into<String>,
        goal: impl Into<String>,
        contract_cards: Vec<RankedContractCard>,
        return_shape_expr: Expr,
        return_shape: Arc<dyn Shape>,
        expected_form: Expr,
    ) -> Self {
        Self {
            name,
            source_payload: source_payload.into(),
            goal: goal.into(),
            contract_cards,
            target_codec: Symbol::qualified("codec", "json"),
            return_shape_expr,
            return_shape,
            expected_form,
        }
    }
}

/// Metrics aggregated for one authoring arm.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorArmMetrics {
    /// Prompt payload tokens. This is the primary cost target.
    pub payload_tokens: u64,
    /// Prompt payload bytes. This is secondary accounting.
    pub payload_bytes: u64,
    /// Deterministic model execution calls made by fake runners.
    pub execution_calls: u64,
    /// Route attempts recorded by the authoring loop.
    pub route_attempts: u64,
    /// Declared fake-runner cost accumulated for attempted model routes.
    pub declared_cost: u64,
    /// Cases that reached the configured fake success condition.
    pub accepted_cases: u64,
    /// Deterministic cassette hashes, one per case.
    pub cassette_hashes: Vec<String>,
}

/// Report for a full offline authoring benchmark run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorBenchReport {
    /// Ordered arm metrics.
    pub arms: Vec<(AuthorArm, AuthorArmMetrics)>,
}

impl AuthorBenchReport {
    /// Returns metrics for an arm.
    pub fn metrics(&self, arm: AuthorArm) -> Option<&AuthorArmMetrics> {
        self.arms
            .iter()
            .find_map(|(candidate, metrics)| (candidate == &arm).then_some(metrics))
    }
}

/// Returns the standard offline authoring benchmark arms.
pub fn standard_author_arms() -> Vec<AuthorArm> {
    vec![
        AuthorArm::SourcePayload,
        AuthorArm::ContractPayload,
        AuthorArm::ContractGrammar,
        AuthorArm::Downshifted,
    ]
}

/// Returns the standard offline authoring corpus.
pub fn standard_author_cases() -> Vec<AuthorCase> {
    crate::eval_author_corpus::standard_author_cases()
}

/// Runs the offline authoring benchmark with deterministic fakes only.
pub fn run_author_bench(
    cx: &mut Cx,
    cases: &[AuthorCase],
    arms: &[AuthorArm],
) -> Result<AuthorBenchReport> {
    if cases.is_empty() {
        return Err(Error::Eval("author bench cases are empty".to_owned()));
    }
    if arms.is_empty() {
        return Err(Error::Eval("author bench arms are empty".to_owned()));
    }
    ensure_json_codec(cx)?;

    let mut report = AuthorBenchReport { arms: Vec::new() };
    for arm in arms {
        let mut metrics = AuthorArmMetrics::default();
        for case in cases {
            let measured = run_author_case(cx, case, arm)?;
            metrics.payload_tokens = metrics
                .payload_tokens
                .saturating_add(measured.payload_tokens);
            metrics.payload_bytes = metrics.payload_bytes.saturating_add(measured.payload_bytes);
            metrics.execution_calls = metrics
                .execution_calls
                .saturating_add(measured.execution_calls);
            metrics.route_attempts = metrics
                .route_attempts
                .saturating_add(measured.route_attempts);
            metrics.declared_cost = metrics.declared_cost.saturating_add(measured.declared_cost);
            metrics.accepted_cases = metrics.accepted_cases.saturating_add(measured.accepted);
            metrics.cassette_hashes.push(measured.cassette_hash);
        }
        report.arms.push((arm.clone(), metrics));
    }

    Ok(report)
}

struct MeasuredAuthorCase {
    payload_tokens: u64,
    payload_bytes: u64,
    execution_calls: u64,
    route_attempts: u64,
    declared_cost: u64,
    accepted: u64,
    cassette_hash: String,
}

fn run_author_case(cx: &mut Cx, case: &AuthorCase, arm: &AuthorArm) -> Result<MeasuredAuthorCase> {
    match arm {
        AuthorArm::SourcePayload => source_payload_case(case),
        AuthorArm::ContractPayload => contract_case(cx, case, false, ContractRoute::Payload),
        AuthorArm::ContractGrammar => contract_case(cx, case, true, ContractRoute::Grammar),
        AuthorArm::Downshifted => contract_case(cx, case, true, ContractRoute::Downshift),
    }
}

fn source_payload_case(case: &AuthorCase) -> Result<MeasuredAuthorCase> {
    let payload_tokens = estimate_prompt_tokens(&case.source_payload) as u64;
    let cassette = DevCassette::from_events(
        Symbol::qualified("forge-author-bench", AuthorArm::SourcePayload.name()),
        vec![DevEvent::validate(
            case.name.clone(),
            Expr::Map(vec![
                entry(
                    "arm",
                    Expr::Symbol(Symbol::qualified(
                        "forge-author-arm",
                        AuthorArm::SourcePayload.name(),
                    )),
                ),
                entry("payload-tokens", uint(payload_tokens)),
                entry("route-attempts", uint(1)),
                entry("declared-cost", uint(HIGH_TIER_COST)),
            ]),
        )?],
    )?;
    Ok(MeasuredAuthorCase {
        payload_tokens,
        payload_bytes: case.source_payload.len() as u64,
        execution_calls: 1,
        route_attempts: 1,
        declared_cost: HIGH_TIER_COST,
        accepted: 1,
        cassette_hash: cassette.content_hash().to_owned(),
    })
}

enum ContractRoute {
    Payload,
    Grammar,
    Downshift,
}

fn contract_case(
    cx: &mut Cx,
    case: &AuthorCase,
    strict_grammar: bool,
    route: ContractRoute,
) -> Result<MeasuredAuthorCase> {
    let task = author_task(case, strict_grammar);
    let projection = project_contracts(&task.contract_cards, &task.projection_caps);
    let payload_tokens = projection.tokens as u64;
    let payload_bytes = projection.text.len() as u64;
    let expected = encoded_json(&case.expected_form);
    let malformed = "{not-json".to_owned();

    match route {
        ContractRoute::Payload => {
            let cheap = BenchFabric::new(vec![malformed]);
            let high = BenchFabric::new(vec![expected]);
            let policy = RoutePolicy::new(
                vec![
                    RouteTarget::new("cheap-contract", &cheap),
                    RouteTarget::new("high-contract", &high),
                ],
                1,
            );
            let outcome = run_author_task(cx, &task, &policy)?;
            measured_contract_case(
                payload_tokens,
                payload_bytes,
                &outcome.attempts,
                outcome.checked_form.is_some(),
                outcome.cassette.content_hash(),
                &[
                    ("cheap-contract", CHEAP_COST),
                    ("high-contract", HIGH_TIER_COST),
                ],
                &[&cheap, &high],
            )
        }
        ContractRoute::Grammar => {
            let cheap = BenchFabric::new(vec![expected]);
            let high = BenchFabric::new(vec![encoded_json(&case.expected_form)]);
            let policy = RoutePolicy::new(
                vec![
                    RouteTarget::new("cheap-contract", &cheap),
                    RouteTarget::new("high-contract", &high),
                ],
                1,
            );
            let outcome = run_author_task(cx, &task, &policy)?;
            measured_contract_case(
                payload_tokens,
                payload_bytes,
                &outcome.attempts,
                outcome.checked_form.is_some(),
                outcome.cassette.content_hash(),
                &[
                    ("cheap-contract", CHEAP_COST),
                    ("high-contract", HIGH_TIER_COST),
                ],
                &[&cheap, &high],
            )
        }
        ContractRoute::Downshift => {
            let cheap = BenchFabric::new(vec![malformed]);
            let escalation = BenchFabric::new(vec![expected]);
            let high = BenchFabric::new(vec![encoded_json(&case.expected_form)]);
            let policy = RoutePolicy::new(
                vec![
                    RouteTarget::new("cheap-downshift", &cheap),
                    RouteTarget::new("escalation-downshift", &escalation),
                    RouteTarget::new("high-contract", &high),
                ],
                1,
            );
            let outcome = run_author_task(cx, &task, &policy)?;
            measured_contract_case(
                payload_tokens,
                payload_bytes,
                &outcome.attempts,
                outcome.checked_form.is_some(),
                outcome.cassette.content_hash(),
                &[
                    ("cheap-downshift", CHEAP_COST),
                    ("escalation-downshift", ESCALATION_COST),
                    ("high-contract", HIGH_TIER_COST),
                ],
                &[&cheap, &escalation, &high],
            )
        }
    }
}

fn measured_contract_case(
    payload_tokens: u64,
    payload_bytes: u64,
    attempts: &[RouteAttempt],
    accepted: bool,
    cassette_hash: &str,
    costs: &[(&str, u64)],
    fabrics: &[&BenchFabric],
) -> Result<MeasuredAuthorCase> {
    Ok(MeasuredAuthorCase {
        payload_tokens,
        payload_bytes,
        execution_calls: fabrics
            .iter()
            .map(|fabric| fabric.model_request_count() as u64)
            .sum(),
        route_attempts: attempts.len() as u64,
        declared_cost: attempts
            .iter()
            .filter(|attempt| !matches!(attempt.status, RouteAttemptStatus::Skipped))
            .map(|attempt| target_cost(&attempt.target, costs))
            .sum::<Result<u64>>()?,
        accepted: u64::from(accepted),
        cassette_hash: cassette_hash.to_owned(),
    })
}

fn target_cost(target: &str, costs: &[(&str, u64)]) -> Result<u64> {
    costs
        .iter()
        .find_map(|(id, cost)| (*id == target).then_some(*cost))
        .ok_or_else(|| Error::Eval(format!("author bench target {target} has no declared cost")))
}

fn author_task(case: &AuthorCase, strict_grammar: bool) -> AuthorTask {
    let mut projection_caps =
        ContractProjectionCaps::new(case.target_codec.clone(), usize::MAX / 4);
    projection_caps.include_examples = false;
    AuthorTask {
        name: case.name.clone(),
        goal: case.goal.clone(),
        target_codec: case.target_codec.clone(),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: case.contract_cards.len(),
        },
        contract_cards: case.contract_cards.clone(),
        projection_caps,
        return_shape_expr: case.return_shape_expr.clone(),
        return_shape: case.return_shape.clone(),
        verifiers: Vec::new(),
        strict_grammar,
    }
}

fn ensure_json_codec(cx: &mut Cx) -> Result<()> {
    if cx
        .registry()
        .codec_by_symbol(&Symbol::qualified("codec", "json"))
        .is_some()
    {
        return Ok(());
    }
    let json = sim_codec_json::JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json)?;
    Ok(())
}

fn encoded_json(expr: &Expr) -> String {
    sim_codec_json::expr_to_json(expr).to_string()
}

struct BenchFabric {
    model_outputs: Mutex<Vec<String>>,
    model_requests: Mutex<usize>,
    realize_requests: Mutex<usize>,
}

impl BenchFabric {
    fn new(model_outputs: Vec<String>) -> Self {
        Self {
            model_outputs: Mutex::new(model_outputs),
            model_requests: Mutex::new(0),
            realize_requests: Mutex::new(0),
        }
    }

    fn model_request_count(&self) -> usize {
        *self
            .model_requests
            .lock()
            .expect("model request count lock")
    }
}

impl EvalFabric for BenchFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        if ModelRequest::try_from(request.expr.clone()).is_ok() {
            *self
                .model_requests
                .lock()
                .expect("model request count lock") += 1;
            let text = {
                let mut outputs = self.model_outputs.lock().expect("model output lock");
                if outputs.is_empty() {
                    return Err(Error::Eval(
                        "author bench fake runner is exhausted".to_owned(),
                    ));
                }
                outputs.remove(0)
            };
            let response = ModelResponse::new(
                Symbol::qualified("runner", "author-bench"),
                "author-bench",
                vec![text_content(text)],
                Symbol::new("stop"),
            );
            return Ok(EvalReply {
                value: cx.factory().expr(Expr::from(response))?,
                diagnostics: Vec::new(),
                trace: None,
            });
        }

        *self
            .realize_requests
            .lock()
            .expect("realize request count lock") += 1;
        Ok(EvalReply {
            value: cx.factory().expr(request.expr)?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn uint(value: u64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("number", "u64"),
        canonical: value.to_string(),
    })
}
