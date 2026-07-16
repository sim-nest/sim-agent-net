use sim_codec_bridge::{
    ask_profile_symbol, brief_profile_symbol, collab_profile_symbol, loom_profile_symbol,
};
use sim_kernel::{Cx, Error, Expr, NumberLiteral, Result, Symbol};
use sim_value::build::entry;

use crate::{CompiledIntent, Verifier, VerifyCatalog};

/// Recorded model answer and token cost for one deterministic eval playback.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalPlayback {
    /// Answer returned by the recorded model playback.
    pub answer: Expr,
    /// Token cost charged to this playback.
    pub tokens: u64,
}

impl EvalPlayback {
    /// Builds one recorded playback row.
    pub fn new(answer: Expr, tokens: u64) -> Self {
        Self { answer, tokens }
    }
}

/// Deterministic cassette rows for one eval case.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalCassette {
    /// Compiler or lift token cost charged when the artifact is not cached.
    pub compiler_tokens: u64,
    /// Raw prose baseline playback.
    pub raw: EvalPlayback,
    /// Compiled artifact playback.
    pub compiled: EvalPlayback,
    /// Downshifted compiled playback.
    pub downshifted: EvalPlayback,
}

impl EvalCassette {
    /// Builds the deterministic playback set for one case.
    pub fn new(
        compiler_tokens: u64,
        raw: EvalPlayback,
        compiled: EvalPlayback,
        downshifted: EvalPlayback,
    ) -> Self {
        Self {
            compiler_tokens,
            raw,
            compiled,
            downshifted,
        }
    }
}

/// One committed FORGE eval case.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalCase {
    /// Stable case id.
    pub name: Symbol,
    /// BRIDGE profile covered by this case.
    pub profile: Symbol,
    /// Source prose under test.
    pub prose: String,
    /// Concrete arguments supplied to the compiled intent.
    pub args: Expr,
    /// Ground-truth answer expected by the verifier set.
    pub gold_answer: Expr,
    /// Semantic verifier ids that must pass for this case.
    pub verifiers: Vec<Symbol>,
    /// Recorded, network-free model playbacks for this case.
    pub cassette: EvalCassette,
}

impl EvalCase {
    /// Builds one eval case.
    pub fn new(
        name: Symbol,
        profile: Symbol,
        prose: impl Into<String>,
        args: Expr,
        gold_answer: Expr,
        verifiers: Vec<Symbol>,
        cassette: EvalCassette,
    ) -> Self {
        Self {
            name,
            profile,
            prose: prose.into(),
            args,
            gold_answer,
            verifiers,
            cassette,
        }
    }
}

/// One eval arm configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalArm {
    /// Stable report name.
    pub name: String,
    /// Whether this arm runs a compiled artifact instead of raw prose.
    pub compiled: bool,
    /// Whether this arm uses a golden artifact cache hit.
    pub cached: bool,
    /// Whether this arm uses the downshifted execution playback.
    pub downshift: bool,
    /// Whether this arm is served entirely by identical-request replay.
    pub replay: bool,
}

impl EvalArm {
    /// Raw prose baseline arm.
    pub fn raw_prose(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            compiled: false,
            cached: false,
            downshift: false,
            replay: false,
        }
    }

    /// Compiled arm that pays the lift/compiler cost for every case.
    pub fn compiled_uncached(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            compiled: true,
            cached: false,
            downshift: false,
            replay: false,
        }
    }

    /// Compiled arm that hits a golden artifact and pays no compiler cost.
    pub fn compiled_cached(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            compiled: true,
            cached: true,
            downshift: false,
            replay: false,
        }
    }

    /// Compiled cached arm that executes on the downshifted playback.
    pub fn compiled_cached_downshifted(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            compiled: true,
            cached: true,
            downshift: true,
            replay: false,
        }
    }

    /// Identical-request replay arm.
    pub fn identical_request_replay(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            compiled: true,
            cached: true,
            downshift: false,
            replay: true,
        }
    }
}

/// Metrics aggregated for one eval arm.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArmMetrics {
    /// Fraction of cases whose semantic verifiers passed.
    pub accuracy: f64,
    /// Total token cost recorded by the arm.
    pub tokens: u64,
    /// Lift/compiler model calls made by the arm.
    pub compiler_calls: u64,
    /// Execution model calls made by the arm.
    pub execution_calls: u64,
    /// Identical-request replay hits recorded by the arm.
    pub replay_hits: u64,
}

/// Eval report across all requested arms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvalReport {
    /// Ordered arm metrics.
    pub arms: Vec<(String, ArmMetrics)>,
}

impl EvalReport {
    /// Returns metrics for a named arm.
    pub fn metrics(&self, name: &str) -> Option<&ArmMetrics> {
        self.arms
            .iter()
            .find_map(|(arm, metrics)| (arm == name).then_some(metrics))
    }
}

/// Returns the standard FORGE eval arms.
pub fn standard_eval_arms() -> Vec<EvalArm> {
    vec![
        EvalArm::raw_prose("raw-prose-baseline"),
        EvalArm::compiled_uncached("compiled-uncached"),
        EvalArm::compiled_cached("compiled-cached"),
        EvalArm::compiled_cached_downshifted("compiled-cached-downshifted"),
        EvalArm::identical_request_replay("identical-request-replay"),
    ]
}

/// Returns the committed network-free FORGE eval corpus.
pub fn standard_eval_corpus() -> Vec<EvalCase> {
    vec![
        corpus_case(CorpusSpec {
            id: "brief-status",
            profile: brief_profile_symbol(),
            prose: "Summarize the release status in one word.",
            args: Expr::String("release status is green".to_owned()),
            gold_answer: Expr::String("green".to_owned()),
            tokens: TokenSpec::new(42, 118, 64, 31),
        }),
        corpus_case(CorpusSpec {
            id: "ask-count",
            profile: ask_profile_symbol(),
            prose: "Count the accepted review checks.",
            args: Expr::Vector(vec![int(1), int(1), int(1), int(1)]),
            gold_answer: int(4),
            tokens: TokenSpec::new(36, 104, 58, 28),
        }),
        corpus_case(CorpusSpec {
            id: "loom-plan",
            profile: loom_profile_symbol(),
            prose: "Order the two rollout rows.",
            args: Expr::Vector(vec![
                Expr::String("audit".to_owned()),
                Expr::String("ship".to_owned()),
            ]),
            gold_answer: Expr::Vector(vec![
                Expr::String("audit".to_owned()),
                Expr::String("ship".to_owned()),
            ]),
            tokens: TokenSpec::new(48, 132, 72, 36),
        }),
        corpus_case(CorpusSpec {
            id: "collab-vote",
            profile: collab_profile_symbol(),
            prose: "Approve only if the verifier passes.",
            args: Expr::Bool(true),
            gold_answer: Expr::Bool(true),
            tokens: TokenSpec::new(44, 126, 66, 33),
        }),
    ]
}

/// Runs the corpus against the requested eval arms.
pub fn run_eval(cx: &mut Cx, corpus: &[EvalCase], arms: &[EvalArm]) -> Result<EvalReport> {
    if corpus.is_empty() {
        return Err(Error::Eval("eval corpus is empty".to_owned()));
    }
    if arms.is_empty() {
        return Err(Error::Eval("eval arms are empty".to_owned()));
    }

    let mut report = EvalReport { arms: Vec::new() };
    for arm in arms {
        let mut metrics = ArmMetrics::default();
        let mut passed = 0_u64;
        for case in corpus {
            let outcome = run_case(cx, case, arm)?;
            metrics.tokens = metrics.tokens.saturating_add(outcome.tokens);
            metrics.compiler_calls = metrics
                .compiler_calls
                .saturating_add(outcome.compiler_calls);
            metrics.execution_calls = metrics
                .execution_calls
                .saturating_add(outcome.execution_calls);
            metrics.replay_hits = metrics.replay_hits.saturating_add(outcome.replay_hits);
            if outcome.passed {
                passed = passed.saturating_add(1);
            }
        }
        metrics.accuracy = passed as f64 / corpus.len() as f64;
        report.arms.push((arm.name.clone(), metrics));
    }
    Ok(report)
}

struct CaseOutcome {
    passed: bool,
    tokens: u64,
    compiler_calls: u64,
    execution_calls: u64,
    replay_hits: u64,
}

fn run_case(cx: &mut Cx, case: &EvalCase, arm: &EvalArm) -> Result<CaseOutcome> {
    if case.verifiers.is_empty() {
        return Err(Error::Eval(format!(
            "eval case {} declares no verifiers",
            case.name
        )));
    }

    let mut tokens = 0_u64;
    let mut compiler_calls = 0_u64;
    let mut execution_calls = 0_u64;
    let mut replay_hits = 0_u64;
    if arm.compiled && !arm.cached && !arm.replay {
        compiler_calls = 1;
        tokens = tokens.saturating_add(case.cassette.compiler_tokens);
    }

    let answer = if arm.replay {
        replay_hits = 1;
        &case.cassette.compiled.answer
    } else {
        execution_calls = 1;
        let playback = playback_for(case, arm);
        tokens = tokens.saturating_add(playback.tokens);
        &playback.answer
    };

    Ok(CaseOutcome {
        passed: verify_case(cx, case, answer)?,
        tokens,
        compiler_calls,
        execution_calls,
        replay_hits,
    })
}

fn playback_for<'a>(case: &'a EvalCase, arm: &EvalArm) -> &'a EvalPlayback {
    if !arm.compiled {
        &case.cassette.raw
    } else if arm.downshift {
        &case.cassette.downshifted
    } else {
        &case.cassette.compiled
    }
}

fn verify_case(cx: &mut Cx, case: &EvalCase, answer: &Expr) -> Result<bool> {
    let mut catalog = VerifyCatalog::new();
    for verifier in &case.verifiers {
        catalog.register_verifier(
            verifier.clone(),
            Verifier::Assertion {
                predicate: equals_predicate(case.gold_answer.clone()),
            },
        );
    }
    let intent = CompiledIntent {
        name: case.name.clone(),
        verifiers: case.verifiers.clone(),
        ..CompiledIntent::default()
    };
    Ok(catalog.verify_answer(cx, &intent, answer)?.accepted())
}

fn equals_predicate(expected: Expr) -> Expr {
    Expr::Map(vec![
        entry(
            "predicate",
            Expr::Symbol(Symbol::qualified("forge", "equals")),
        ),
        entry("expected", expected),
    ])
}

struct CorpusSpec {
    id: &'static str,
    profile: Symbol,
    prose: &'static str,
    args: Expr,
    gold_answer: Expr,
    tokens: TokenSpec,
}

struct TokenSpec {
    compiler: u64,
    raw: u64,
    compiled: u64,
    downshifted: u64,
}

impl TokenSpec {
    fn new(compiler: u64, raw: u64, compiled: u64, downshifted: u64) -> Self {
        Self {
            compiler,
            raw,
            compiled,
            downshifted,
        }
    }
}

fn corpus_case(spec: CorpusSpec) -> EvalCase {
    EvalCase::new(
        Symbol::qualified("forge/eval", spec.id),
        spec.profile,
        spec.prose,
        spec.args,
        spec.gold_answer.clone(),
        vec![Symbol::qualified(
            "forge/eval",
            format!("{}-gold", spec.id).as_str(),
        )],
        EvalCassette::new(
            spec.tokens.compiler,
            EvalPlayback::new(spec.gold_answer.clone(), spec.tokens.raw),
            EvalPlayback::new(spec.gold_answer.clone(), spec.tokens.compiled),
            EvalPlayback::new(spec.gold_answer, spec.tokens.downshifted),
        ),
    )
}

fn int(value: i64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("number", "i64"),
        canonical: value.to_string(),
    })
}
