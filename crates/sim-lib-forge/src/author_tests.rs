use std::sync::{Arc, Mutex};

use sim_kernel::{
    Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, MatchScore, Result, ShapeDoc, ShapeMatch,
    Symbol, Value, shape::Shape, testing::bare_cx as cx,
};
use sim_lib_agent_runner_core::{
    FENCE_DATA_RULE, ModelRequest, ModelResponse, OUTPUT_GRAMMAR_DIALECT_EXTRA,
    OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
};
use sim_shape::{ExprKind, ExprKindShape};
use sim_value::{access::field, build::entry};

use crate::{
    AuthorTask, CONTRACT_PROJECTION_EXTRA, ContractCard, ContractProjection,
    ContractProjectionCaps, OUTPUT_GRAMMAR_GRAPH_EXTRA, RankedContractCard, RouteAttemptStatus,
    RoutePolicy, RouteTarget, ShapeQuery, author_model_request, authorized_capabilities,
    estimate_prompt_tokens, project_contracts, run_author_task,
};

#[test]
fn projection_reduces_and_drops_without_exceeding_budget() {
    let cards = vec![
        ranked_card(
            "first",
            "set key",
            Some(Expr::Call {
                operator: Box::new(Expr::Symbol(Symbol::qualified("table", "set"))),
                args: vec![
                    Expr::Symbol(Symbol::new("table")),
                    Expr::Symbol(Symbol::new("alpha")),
                    Expr::String("value".to_owned()),
                ],
            }),
        ),
        ranked_card(
            "second",
            "return entries",
            Some(Expr::String("large".repeat(8))),
        ),
    ];
    let caps = ContractProjectionCaps::new(Symbol::qualified("codec", "lisp"), 12);

    let projection = project_contracts(&cards, &caps);

    assert!(projection.tokens <= caps.token_budget);
    assert_eq!(projection.included, 1);
    assert_eq!(projection.summary_only, 1);
    assert_eq!(projection.dropped, 1);
    assert!(projection.text.contains("contract: fixture/first"));
    assert!(!projection.text.contains("args-shape"));
    assert!(!projection.text.contains("fixture/second"));
    assert!(
        projection
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("summary only"))
    );
    assert!(
        projection
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("dropped fixture/second"))
    );
}

#[test]
fn strict_author_request_refuses_shapes_without_sg3_graph() {
    let mut cx = cx();
    let projection = projection("contract: fixture/echo\nsummary: return text");
    let task = AuthorTask {
        name: Symbol::qualified("forge", "unsupported"),
        goal: "Return checked data.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: 1,
        },
        contract_cards: Vec::new(),
        projection_caps: ContractProjectionCaps::new(Symbol::qualified("codec", "json"), 128),
        return_shape_expr: Expr::Symbol(Symbol::qualified("fixture", "Unsupported")),
        return_shape: Arc::new(UnsupportedShape),
        verifiers: Vec::new(),
        strict_grammar: true,
    };

    let err = author_model_request(&mut cx, &task, &projection).unwrap_err();

    assert!(
        err.to_string()
            .contains("strict grammar cannot lower return shape")
    );
}

#[test]
fn author_request_carries_output_grammar_and_graph_metadata() {
    let mut cx = cx();
    let projection = projection("contract: fixture/echo\nsummary: return text");
    let task = AuthorTask {
        name: Symbol::qualified("forge", "echo"),
        goal: "Return a string answer.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: 1,
        },
        contract_cards: Vec::new(),
        projection_caps: ContractProjectionCaps::new(Symbol::qualified("codec", "json"), 128),
        return_shape_expr: Expr::Symbol(Symbol::qualified("shape", "String")),
        return_shape: Arc::new(ExprKindShape::new(ExprKind::String)),
        verifiers: Vec::new(),
        strict_grammar: true,
    };

    let request = author_model_request(&mut cx, &task, &projection).unwrap();

    assert!(fenced_task_projection(&request).contains(FENCE_DATA_RULE));
    assert!(fenced_task_projection(&request).contains("contract: fixture/echo"));
    assert!(matches!(
        extra(&request, CONTRACT_PROJECTION_EXTRA),
        Some(Expr::String(text)) if text.contains(FENCE_DATA_RULE)
    ));
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_EXTRA),
        Some(&Expr::String(r#"{"type":"string"}"#.to_owned()))
    );
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_DIALECT_EXTRA),
        Some(&Expr::Symbol(Symbol::new("json-schema")))
    );
    assert_eq!(
        extra(&request, OUTPUT_GRAMMAR_REQUIRED_EXTRA),
        Some(&Expr::Bool(true))
    );
    let graph = extra(&request, OUTPUT_GRAMMAR_GRAPH_EXTRA).unwrap();
    assert_eq!(
        field(graph, "kind"),
        Some(&Expr::Symbol(Symbol::qualified(
            "forge",
            "OutputGrammarGraph"
        )))
    );
    assert_eq!(
        field(graph, "root"),
        Some(&Expr::Symbol(Symbol::qualified(
            "grammar-production",
            "terminal"
        )))
    );
}

#[test]
fn authorized_capabilities_dedupes_projected_cards() {
    let cards = vec![
        ranked_card("first", "set key", None),
        ranked_card("second", "get key", None),
    ];

    let caps = authorized_capabilities(&cards);

    assert_eq!(caps, vec![Symbol::qualified("capability", "table")]);
}

#[test]
fn author_task_loop_accepts_cheap_target_without_escalation() {
    let mut cx = author_cx();
    let cheap = ScriptedAuthorFabric::new(vec![encoded_json(&Expr::String("ok".to_owned()))]);
    let strong =
        ScriptedAuthorFabric::new(vec![encoded_json(&Expr::String("fallback".to_owned()))]);
    let task = string_author_task(vec![ranked_card("echo", "return text", None)]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("cheap", &cheap),
            RouteTarget::new("strong", &strong),
        ],
        1,
    );

    let outcome = run_author_task(&mut cx, &task, &policy).unwrap();

    assert_eq!(outcome.checked_form, Some(Expr::String("ok".to_owned())));
    assert_eq!(outcome.realized, Some(Expr::String("ok".to_owned())));
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].status, RouteAttemptStatus::Accepted);
    assert_eq!(cheap.model_request_count(), 1);
    assert_eq!(cheap.realize_count(), 1);
    assert_eq!(strong.model_request_count(), 0);
    assert!(!outcome.cassette.content_hash().is_empty());
}

#[test]
fn author_task_loop_escalates_after_malformed_output() {
    let mut cx = author_cx();
    let cheap = ScriptedAuthorFabric::new(vec!["{not-json".to_owned()]);
    let strong = ScriptedAuthorFabric::new(vec![encoded_json(&Expr::String("ok".to_owned()))]);
    let task = string_author_task(vec![ranked_card("echo", "return text", None)]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("cheap", &cheap),
            RouteTarget::new("strong", &strong),
        ],
        1,
    );

    let outcome = run_author_task(&mut cx, &task, &policy).unwrap();

    assert_eq!(outcome.checked_form, Some(Expr::String("ok".to_owned())));
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].status, RouteAttemptStatus::Failed);
    assert_eq!(outcome.attempts[1].status, RouteAttemptStatus::Accepted);
    assert_eq!(cheap.realize_count(), 0);
    assert_eq!(strong.realize_count(), 1);
}

#[test]
fn author_task_loop_exhaustion_returns_diagnostic_outcome() {
    let mut cx = author_cx();
    let cheap = ScriptedAuthorFabric::new(vec!["{not-json".to_owned()]);
    let task = string_author_task(vec![ranked_card("echo", "return text", None)]);
    let policy = RoutePolicy::new(vec![RouteTarget::new("cheap", &cheap)], 1);

    let outcome = run_author_task(&mut cx, &task, &policy).unwrap();

    assert_eq!(outcome.checked_form, None);
    assert_eq!(outcome.realized, None);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].status, RouteAttemptStatus::Failed);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("exhausted route policy"))
    );
    assert!(!outcome.cassette.content_hash().is_empty());
}

#[test]
fn author_task_loop_refuses_out_of_ceiling_capability() {
    let mut cx = author_cx();
    let cheap = ScriptedAuthorFabric::new(vec![encoded_json(&Expr::Call {
        operator: Box::new(Expr::Symbol(Symbol::qualified("capability", "fs-write"))),
        args: Vec::new(),
    })]);
    let mut task = any_author_task(vec![ranked_card("echo", "return text", None)]);
    task.strict_grammar = false;
    let policy = RoutePolicy::new(vec![RouteTarget::new("cheap", &cheap)], 1);

    let outcome = run_author_task(&mut cx, &task, &policy).unwrap();

    assert_eq!(outcome.checked_form, None);
    assert_eq!(outcome.realized, None);
    assert_eq!(cheap.realize_count(), 0);
    assert!(matches!(
        outcome.attempts.first().and_then(|attempt| attempt.reason.as_ref()),
        Some(reason) if reason.contains("outside projection ceiling")
    ));
}

fn ranked_card(name: &str, summary: &str, example: Option<Expr>) -> RankedContractCard {
    RankedContractCard {
        card: ContractCard {
            lib: Symbol::qualified("fixture", "contracts"),
            export_kind: Symbol::qualified("export", "function"),
            symbol: Symbol::qualified("fixture", name),
            args_shape: Some(Expr::Symbol(Symbol::qualified("shape", "List"))),
            result_shape: Some(Expr::Symbol(Symbol::qualified("shape", "String"))),
            capability_symbols: vec![Symbol::qualified("capability", "table")],
            card_requires: None,
            summary: summary.to_owned(),
            example,
            partial: Vec::new(),
        },
        score: 10,
        reasons: vec!["fixture match".to_owned()],
    }
}

fn string_author_task(cards: Vec<RankedContractCard>) -> AuthorTask {
    AuthorTask {
        name: Symbol::qualified("forge", "author-fixture"),
        goal: "Return a checked string.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: cards.len(),
        },
        contract_cards: cards,
        projection_caps: ContractProjectionCaps::new(Symbol::qualified("codec", "json"), 256),
        return_shape_expr: Expr::Symbol(Symbol::qualified("shape", "String")),
        return_shape: Arc::new(ExprKindShape::new(ExprKind::String)),
        verifiers: Vec::new(),
        strict_grammar: true,
    }
}

fn any_author_task(cards: Vec<RankedContractCard>) -> AuthorTask {
    AuthorTask {
        name: Symbol::qualified("forge", "author-fixture"),
        goal: "Return a checked form.".to_owned(),
        target_codec: Symbol::qualified("codec", "json"),
        query: ShapeQuery {
            args: None,
            result: None,
            limit: cards.len(),
        },
        contract_cards: cards,
        projection_caps: ContractProjectionCaps::new(Symbol::qualified("codec", "json"), 256),
        return_shape_expr: Expr::Symbol(Symbol::qualified("shape", "Any")),
        return_shape: Arc::new(UnsupportedShape),
        verifiers: Vec::new(),
        strict_grammar: false,
    }
}

fn projection(text: &str) -> ContractProjection {
    ContractProjection {
        text: text.to_owned(),
        tokens: estimate_prompt_tokens(text),
        included: 1,
        summary_only: 0,
        dropped: 0,
        diagnostics: Vec::new(),
    }
}

fn author_cx() -> Cx {
    let mut cx = cx();
    let json = sim_codec_json::JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    cx
}

fn encoded_json(expr: &Expr) -> String {
    sim_codec_json::expr_to_json(expr).to_string()
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn fenced_task_projection(request: &ModelRequest) -> &str {
    match field(&request.task, "contract-projection") {
        Some(Expr::String(text)) => text,
        other => panic!("expected fenced contract projection, got {other:?}"),
    }
}

fn extra<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a Expr> {
    request.extra.iter().find_map(|(key, value)| {
        if *key == Expr::Symbol(Symbol::new(name)) {
            Some(value)
        } else {
            None
        }
    })
}

struct ScriptedAuthorFabric {
    model_outputs: Mutex<Vec<String>>,
    model_requests: Mutex<usize>,
    realize_requests: Mutex<Vec<Expr>>,
}

impl ScriptedAuthorFabric {
    fn new(model_outputs: Vec<String>) -> Self {
        Self {
            model_outputs: Mutex::new(model_outputs),
            model_requests: Mutex::new(0),
            realize_requests: Mutex::new(Vec::new()),
        }
    }

    fn model_request_count(&self) -> usize {
        *self.model_requests.lock().unwrap()
    }

    fn realize_count(&self) -> usize {
        self.realize_requests.lock().unwrap().len()
    }
}

impl EvalFabric for ScriptedAuthorFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        if ModelRequest::try_from(request.expr.clone()).is_ok() {
            *self.model_requests.lock().unwrap() += 1;
            let text = {
                let mut outputs = self.model_outputs.lock().unwrap();
                if outputs.is_empty() {
                    return Err(Error::Eval(
                        "scripted author fabric is exhausted".to_owned(),
                    ));
                }
                outputs.remove(0)
            };
            let response = ModelResponse::new(
                Symbol::qualified("runner", "author-fixture"),
                "author-fixture",
                vec![text_content(text)],
                Symbol::new("stop"),
            );
            return Ok(EvalReply {
                value: cx.factory().expr(Expr::from(response))?,
                diagnostics: Vec::new(),
                trace: None,
            });
        }

        self.realize_requests
            .lock()
            .unwrap()
            .push(request.expr.clone());
        Ok(EvalReply {
            value: cx.factory().expr(request.expr)?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

struct UnsupportedShape;

impl Shape for UnsupportedShape {
    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol::qualified("fixture", "UnsupportedShape"))
    }

    fn check_value(&self, _cx: &mut Cx, _value: Value) -> Result<ShapeMatch> {
        Ok(ShapeMatch::accept(MatchScore::exact(1)))
    }

    fn check_expr(&self, _cx: &mut Cx, _expr: &Expr) -> Result<ShapeMatch> {
        Ok(ShapeMatch::accept(MatchScore::exact(1)))
    }

    fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new("unsupported"))
    }
}
