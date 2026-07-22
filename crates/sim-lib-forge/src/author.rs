//! Contract projection and grammar-bearing author requests.

use std::sync::Arc;

use sim_kernel::{Cx, Diagnostic, Error, Expr, Result, ShapeRef, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, OutputContract, fenced_data_text};
use sim_shape::{GrammarGraph, Production, Shape, shape_grammar_graph};
use sim_value::build::{entry, uint};

use crate::{RankedContractCard, ShapeQuery, estimate_prompt_tokens};

/// Model-request extension key for the fenced projected contract payload.
pub const CONTRACT_PROJECTION_EXTRA: &str = "forge-contract-projection";
/// Model-request extension key for source SG3 grammar graph metadata.
pub const OUTPUT_GRAMMAR_GRAPH_EXTRA: &str = "output-grammar-graph";

/// Limits and format switches for contract projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractProjectionCaps {
    /// Maximum estimated prompt tokens allowed in the rendered projection.
    pub token_budget: usize,
    /// Whether examples may be included when a full card fits.
    pub include_examples: bool,
    /// Codec surface the projected contracts are intended to help author.
    pub codec: Symbol,
}

impl ContractProjectionCaps {
    /// Builds projection limits for a target codec and token budget.
    pub fn new(codec: Symbol, token_budget: usize) -> Self {
        Self {
            token_budget,
            include_examples: true,
            codec,
        }
    }
}

/// A token-counted model-facing projection of ranked contract cards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractProjection {
    /// Complete rendered projection text that fits the configured budget.
    pub text: String,
    /// Estimated prompt tokens in [`Self::text`].
    pub tokens: usize,
    /// Cards retained in any representation.
    pub included: usize,
    /// Cards reduced all the way to summary-only form.
    pub summary_only: usize,
    /// Cards dropped because even their summary-only form did not fit.
    pub dropped: usize,
    /// Non-fatal reduction and drop diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl ContractProjection {
    /// Encodes projection metadata as open model-request data.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            entry(
                "kind",
                Expr::Symbol(Symbol::qualified("forge", "ContractProjection")),
            ),
            entry("tokens", uint(self.tokens as u64)),
            entry("included", uint(self.included as u64)),
            entry("summary-only", uint(self.summary_only as u64)),
            entry("dropped", uint(self.dropped as u64)),
            entry("text", Expr::String(self.text.clone())),
            entry(
                "diagnostics",
                Expr::List(
                    self.diagnostics
                        .iter()
                        .map(|diagnostic| Expr::String(diagnostic.message.clone()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// One contract-native authoring task to send to a model runner.
#[derive(Clone)]
pub struct AuthorTask {
    /// Stable task name for routing, diagnostics, and cassette rows.
    pub name: Symbol,
    /// Human goal the authoring request must satisfy.
    pub goal: String,
    /// Codec the model must use for the returned checked form.
    pub target_codec: Symbol,
    /// Shape query used to retrieve the projected contract cards.
    pub query: ShapeQuery,
    /// Ranked contract cards available for the task projection.
    pub contract_cards: Vec<RankedContractCard>,
    /// Token and example limits used when projecting contract cards.
    pub projection_caps: ContractProjectionCaps,
    /// Normalized expression naming or constructing the return Shape.
    pub return_shape_expr: Expr,
    /// Shape the model's returned form must satisfy and the grammar is derived from.
    pub return_shape: Arc<dyn Shape>,
    /// Semantic verifier ids that must accept the realized form.
    pub verifiers: Vec<Symbol>,
    /// Whether grammar-constrained output is mandatory.
    pub strict_grammar: bool,
}

/// Projects ranked contract cards into a bounded, token-counted prompt payload.
pub fn project_contracts(
    cards: &[RankedContractCard],
    caps: &ContractProjectionCaps,
) -> ContractProjection {
    project_contracts_with_cards(cards, caps).0
}

pub(crate) fn project_contracts_with_cards(
    cards: &[RankedContractCard],
    caps: &ContractProjectionCaps,
) -> (ContractProjection, Vec<RankedContractCard>) {
    let mut parts = Vec::new();
    let mut projected_cards = Vec::new();
    let mut included = 0usize;
    let mut summary_only = 0usize;
    let mut dropped = 0usize;
    let mut diagnostics = Vec::new();

    for ranked in cards {
        let mut chosen = None;
        for detail in ProjectionDetail::candidates(ranked, caps.include_examples) {
            let rendered = render_ranked_card(ranked, detail);
            let candidate_text = append_projection_part(&parts, &rendered);
            if estimate_prompt_tokens(&candidate_text) <= caps.token_budget {
                chosen = Some((detail, rendered));
                break;
            }
        }

        match chosen {
            Some((ProjectionDetail::SummaryOnly, rendered)) => {
                diagnostics.push(Diagnostic::info(format!(
                    "contract projection reduced {} to summary only under token budget",
                    ranked.card.symbol
                )));
                parts.push(rendered);
                projected_cards.push(ranked.clone());
                included += 1;
                summary_only += 1;
            }
            Some((_, rendered)) => {
                parts.push(rendered);
                projected_cards.push(ranked.clone());
                included += 1;
            }
            None => {
                diagnostics.push(Diagnostic::info(format!(
                    "contract projection dropped {} under token budget",
                    ranked.card.symbol
                )));
                dropped += 1;
            }
        }
    }

    let text = parts.join("\n\n");
    (
        ContractProjection {
            tokens: estimate_prompt_tokens(&text),
            text,
            included,
            summary_only,
            dropped,
            diagnostics,
        },
        projected_cards,
    )
}

/// Builds a grammar-bearing model request from a task and projected contracts.
pub fn author_model_request(
    _cx: &mut Cx,
    task: &AuthorTask,
    projection: &ContractProjection,
) -> Result<ModelRequest> {
    let shape = task.return_shape.as_ref();
    let shape_expr = task.return_shape_expr.clone();

    if task.strict_grammar {
        shape_grammar_graph(shape).map_err(|err| {
            Error::Eval(format!(
                "forge author request strict grammar cannot lower return shape: {err}"
            ))
        })?;
    }

    let output = OutputContract::for_shape(
        task.target_codec.clone(),
        shape_expr.clone(),
        shape,
        task.strict_grammar,
    );
    let graph_metadata = output.grammar_graph.as_ref().map(grammar_graph_expr);

    let projection_expr = projection.to_expr();
    let fenced_projection =
        fenced_data_text("contract-projection", &projection.text, &projection_expr)?;
    let mut request = ModelRequest::new(
        Expr::Map(vec![
            entry(
                "kind",
                Expr::Symbol(Symbol::qualified("forge", "AuthorRequest")),
            ),
            entry("name", Expr::Symbol(task.name.clone())),
            entry("goal", Expr::String(task.goal.clone())),
            entry("target-codec", Expr::Symbol(task.target_codec.clone())),
            entry("contract-query", query_expr(&task.query)),
            entry("return-shape", shape_expr),
            entry("strict-grammar", Expr::Bool(task.strict_grammar)),
            entry(
                "contract-projection",
                Expr::String(fenced_projection.clone()),
            ),
        ]),
        Vec::new(),
    );

    request.extra.push(entry(
        "forge-mode",
        Expr::Symbol(Symbol::qualified("forge", "author-request")),
    ));
    request.extra.push(entry(
        CONTRACT_PROJECTION_EXTRA,
        Expr::String(fenced_projection),
    ));
    request
        .extra
        .push(entry("forge-contract-projection-stats", projection_expr));
    output.into_extra_entries(&mut request.extra);
    if let Some(metadata) = graph_metadata {
        request
            .extra
            .push(entry(OUTPUT_GRAMMAR_GRAPH_EXTRA, metadata));
    }

    Ok(request)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectionDetail {
    FullWithExample,
    ShapeSummary,
    SummaryOnly,
}

impl ProjectionDetail {
    fn candidates(ranked: &RankedContractCard, include_examples: bool) -> Vec<Self> {
        let mut candidates = Vec::new();
        if include_examples && ranked.card.example.is_some() {
            candidates.push(Self::FullWithExample);
        }
        candidates.push(Self::ShapeSummary);
        candidates.push(Self::SummaryOnly);
        candidates
    }
}

fn append_projection_part(parts: &[String], next: &str) -> String {
    if parts.is_empty() {
        next.to_owned()
    } else {
        format!("{}\n\n{next}", parts.join("\n\n"))
    }
}

fn render_ranked_card(ranked: &RankedContractCard, detail: ProjectionDetail) -> String {
    let card = &ranked.card;
    let mut lines = vec![
        format!("contract: {}", card.symbol),
        format!("lib: {}", card.lib),
        format!("kind: {}", card.export_kind),
        format!("score: {}", ranked.score),
        format!("summary: {}", render_summary(&card.summary)),
    ];

    if !matches!(detail, ProjectionDetail::SummaryOnly) {
        lines.push(format!(
            "args-shape: {}",
            render_option_expr(&card.args_shape)
        ));
        lines.push(format!(
            "result-shape: {}",
            render_option_expr(&card.result_shape)
        ));
    }

    if matches!(detail, ProjectionDetail::FullWithExample) {
        lines.push(format!(
            "capabilities: {}",
            render_symbols(&card.capability_symbols)
        ));
        lines.push(format!(
            "card-requires: {}",
            render_option_expr(&card.card_requires)
        ));
        lines.push(format!("example: {}", render_option_expr(&card.example)));
        if !ranked.reasons.is_empty() {
            lines.push(format!("rank-reasons: {}", ranked.reasons.join("; ")));
        }
    }

    lines.join("\n")
}

fn render_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        "none".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn render_symbols(symbols: &[Symbol]) -> String {
    if symbols.is_empty() {
        "none".to_owned()
    } else {
        symbols
            .iter()
            .map(Symbol::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn render_option_expr(expr: &Option<Expr>) -> String {
    expr.as_ref()
        .map(render_expr)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn render_expr(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::Local(symbol) => format!("${symbol}"),
        Expr::String(value) => format!("{value:?}"),
        Expr::Bytes(bytes) => format!("#bytes[{}]", bytes.len()),
        Expr::List(items) => render_sequence("(", ")", items),
        Expr::Vector(items) => render_sequence("[", "]", items),
        Expr::Map(entries) => render_map(entries),
        Expr::Set(items) => render_sequence("#{", "}", items),
        Expr::Call { operator, args } => {
            let mut parts = vec![render_expr(operator)];
            parts.extend(args.iter().map(render_expr));
            format!("({})", parts.join(" "))
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => format!(
            "({} {} {})",
            render_expr(left),
            operator,
            render_expr(right)
        ),
        Expr::Prefix { operator, arg } => format!("({operator} {})", render_expr(arg)),
        Expr::Postfix { operator, arg } => format!("({} {operator})", render_expr(arg)),
        Expr::Block(items) => render_sequence("{", "}", items),
        Expr::Quote { expr, .. } => format!("'{}", render_expr(expr)),
        Expr::Annotated { expr, .. } => render_expr(expr),
        Expr::Extension { tag, payload } => format!("#<{} {}>", tag, render_expr(payload)),
    }
}

fn render_sequence(open: &str, close: &str, items: &[Expr]) -> String {
    let body = items.iter().map(render_expr).collect::<Vec<_>>().join(" ");
    format!("{open}{body}{close}")
}

fn render_map(entries: &[(Expr, Expr)]) -> String {
    let body = entries
        .iter()
        .map(|(key, value)| format!("{}: {}", render_expr(key), render_expr(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{body}}}")
}

fn query_expr(query: &ShapeQuery) -> Expr {
    Expr::Map(vec![
        entry(
            "args",
            query.args.as_ref().map(shape_ref_expr).unwrap_or(Expr::Nil),
        ),
        entry(
            "result",
            query
                .result
                .as_ref()
                .map(shape_ref_expr)
                .unwrap_or(Expr::Nil),
        ),
        entry("limit", uint(query.limit as u64)),
    ])
}

fn shape_ref_expr(shape: &ShapeRef) -> Expr {
    match shape.object().as_shape().and_then(|shape| shape.symbol()) {
        Some(symbol) => Expr::Symbol(symbol),
        None => Expr::String("<anonymous-shape>".to_owned()),
    }
}

fn grammar_graph_expr(graph: &GrammarGraph) -> Expr {
    Expr::Map(vec![
        entry(
            "kind",
            Expr::Symbol(Symbol::qualified("forge", "OutputGrammarGraph")),
        ),
        entry("root", Expr::Symbol(production_kind_symbol(&graph.root))),
        entry("defs", uint(graph.defs.len() as u64)),
        entry("diagnostics", uint(graph.diagnostics.len() as u64)),
    ])
}

fn production_kind_symbol(production: &Production) -> Symbol {
    let name = match production {
        Production::Terminal(_) => "terminal",
        Production::Seq(_) => "seq",
        Production::Alt(_) => "alt",
        Production::Repeat { .. } => "repeat",
        Production::Call { .. } => "call",
        Production::Ref(_) => "ref",
    };
    Symbol::qualified("grammar-production", name)
}
