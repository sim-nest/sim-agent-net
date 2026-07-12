use crate::components::model::{AgentComponent, JudgeBackend, JudgeConfig};
use crate::memory::flatten_expr_text;
use crate::util::value_from_expr;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};
use std::cmp::Ordering;

pub(in crate::components) fn answer_judge(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &JudgeBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, &frame)?;
    let verdict = judge_verdict_expr(cx, backend, request.expr);
    let value = value_from_expr(cx, &verdict)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
}

pub(in crate::components) fn score_candidate(
    _cx: &mut Cx,
    candidate: &Expr,
    config: &JudgeConfig,
) -> f32 {
    score_candidate_details(candidate, config).0
}

fn judge_verdict_expr(cx: &mut Cx, backend: &JudgeBackend, expr: Expr) -> Expr {
    match backend {
        JudgeBackend::Rubric { config } => rubric_verdict_expr(cx, expr, config),
        JudgeBackend::RankedVote { config } => ranked_vote_expr(cx, expr, config),
        JudgeBackend::Threshold { threshold, config } => {
            threshold_verdict_expr(cx, expr, config, *threshold)
        }
    }
}

fn rubric_verdict_expr(cx: &mut Cx, expr: Expr, config: &JudgeConfig) -> Expr {
    let candidate = extract_candidate(&expr);
    let (score, breakdown) = score_candidate_details(&candidate, config);
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("score")),
            number_expr_from_f32(score),
        ),
        (
            Expr::Symbol(Symbol::new("approved")),
            Expr::Bool(score >= 0.5),
        ),
        (
            Expr::Symbol(Symbol::new("candidate")),
            extract_candidate(&expr),
        ),
        (
            Expr::Symbol(Symbol::new("breakdown")),
            breakdown_expr(breakdown),
        ),
        (
            Expr::Symbol(Symbol::new("heuristic")),
            Expr::String(format!(
                "score={:.3}",
                score_candidate(cx, &candidate, config)
            )),
        ),
    ])
}

fn ranked_vote_expr(cx: &mut Cx, expr: Expr, config: &JudgeConfig) -> Expr {
    let candidates = extract_candidates(expr);
    let mut ranked = candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| {
            let (score, breakdown) = score_candidate_details(&candidate, config);
            (index, candidate, score, breakdown)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .2
            .partial_cmp(&left.2)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    let winner = ranked
        .first()
        .map(|(_, candidate, _, _)| candidate.clone())
        .unwrap_or(Expr::Nil);
    let ranking = ranked
        .iter()
        .enumerate()
        .map(|(position, (_, candidate, score, breakdown))| {
            Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("position")),
                    number_expr(position as u32 + 1),
                ),
                (Expr::Symbol(Symbol::new("candidate")), candidate.clone()),
                (
                    Expr::Symbol(Symbol::new("score")),
                    number_expr_from_f32(*score),
                ),
                (
                    Expr::Symbol(Symbol::new("borda")),
                    number_expr((ranked.len().saturating_sub(position)) as u32),
                ),
                (
                    Expr::Symbol(Symbol::new("breakdown")),
                    breakdown_expr(breakdown.clone()),
                ),
            ])
        })
        .collect();
    let top_score = ranked
        .first()
        .map(|(_, candidate, _, _)| score_candidate(cx, candidate, config))
        .unwrap_or(0.0);
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("winner")), winner),
        (Expr::Symbol(Symbol::new("ranking")), Expr::List(ranking)),
        (
            Expr::Symbol(Symbol::new("score")),
            number_expr_from_f32(top_score),
        ),
        (
            Expr::Symbol(Symbol::new("approved")),
            Expr::Bool(!ranked.is_empty()),
        ),
    ])
}

fn threshold_verdict_expr(cx: &mut Cx, expr: Expr, config: &JudgeConfig, threshold: f64) -> Expr {
    let candidate = extract_candidate(&expr);
    let (score, breakdown) = score_candidate_details(&candidate, config);
    let actual = score_candidate(cx, &candidate, config);
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("score")),
            number_expr_from_f32(actual),
        ),
        (
            Expr::Symbol(Symbol::new("approved")),
            Expr::Bool((actual as f64) >= threshold),
        ),
        (
            Expr::Symbol(Symbol::new("threshold")),
            number_expr_from_f32(threshold as f32),
        ),
        (Expr::Symbol(Symbol::new("candidate")), candidate),
        (
            Expr::Symbol(Symbol::new("breakdown")),
            breakdown_expr(breakdown),
        ),
        (
            Expr::Symbol(Symbol::new("weighted-score")),
            number_expr_from_f32(score),
        ),
    ])
}

fn score_candidate_details(candidate: &Expr, config: &JudgeConfig) -> (f32, Vec<(String, f32)>) {
    let mut weighted_sum = 0.0f64;
    let total_weight = config
        .criteria
        .iter()
        .map(|criterion| criterion.weight.max(0.0))
        .sum::<f64>();
    let mut breakdown = Vec::new();
    for criterion in &config.criteria {
        let raw_score = criterion_score(candidate, config, &criterion.name) as f64;
        weighted_sum += raw_score * criterion.weight.max(0.0);
        breakdown.push((criterion.name.clone(), raw_score as f32));
    }
    let score = if total_weight > 0.0 {
        (weighted_sum / total_weight) as f32
    } else {
        0.0
    };
    (score.clamp(0.0, 1.0), breakdown)
}

fn criterion_score(candidate: &Expr, config: &JudgeConfig, name: &str) -> f32 {
    match name {
        "correctness" => correctness_score(candidate, config),
        "style" => style_score(candidate, config),
        "brevity" => brevity_score(candidate),
        other => keyword_overlap_score(candidate, other),
    }
}

fn correctness_score(candidate: &Expr, config: &JudgeConfig) -> f32 {
    let candidate_tokens = tokenize(&flatten_expr_text(candidate));
    if candidate_tokens.is_empty() {
        return 0.0;
    }
    let Some(reference) = &config.reference else {
        return 0.5;
    };
    let reference_tokens = tokenize(reference);
    overlap_ratio(&candidate_tokens, &reference_tokens)
}

fn style_score(candidate: &Expr, config: &JudgeConfig) -> f32 {
    let text = flatten_expr_text(candidate);
    if config.style_markers.is_empty() {
        return 1.0;
    }
    let lowered = text.to_lowercase();
    let hits = config
        .style_markers
        .iter()
        .filter(|marker| lowered.contains(&marker.to_lowercase()))
        .count();
    hits as f32 / config.style_markers.len() as f32
}

fn brevity_score(candidate: &Expr) -> f32 {
    let words = tokenize(&flatten_expr_text(candidate)).len();
    match words {
        0 => 0.0,
        1..=12 => 1.0,
        13..=24 => 0.8,
        25..=36 => 0.5,
        37..=48 => 0.2,
        _ => 0.0,
    }
}

fn keyword_overlap_score(candidate: &Expr, keyword: &str) -> f32 {
    let tokens = tokenize(&flatten_expr_text(candidate));
    if tokens.is_empty() {
        return 0.0;
    }
    let criteria_tokens = tokenize(keyword);
    overlap_ratio(&tokens, &criteria_tokens)
}

fn overlap_ratio(candidate_tokens: &[String], reference_tokens: &[String]) -> f32 {
    if candidate_tokens.is_empty() || reference_tokens.is_empty() {
        return 0.0;
    }
    let hits = reference_tokens
        .iter()
        .filter(|token| candidate_tokens.iter().any(|candidate| candidate == *token))
        .count();
    hits as f32 / reference_tokens.len() as f32
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn extract_candidate(expr: &Expr) -> Expr {
    match expr {
        Expr::Map(entries) => entries
            .iter()
            .find_map(|(key, value)| match key {
                Expr::Symbol(symbol) if symbol.name.as_ref() == "candidate" => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_else(|| expr.clone()),
        _ => expr.clone(),
    }
}

fn extract_candidates(expr: Expr) -> Vec<Expr> {
    match expr {
        Expr::Map(entries) => entries
            .into_iter()
            .find_map(|(key, value)| match key {
                Expr::Symbol(symbol) if symbol.name.as_ref() == "candidates" => Some(value),
                _ => None,
            })
            .map(extract_candidate_list)
            .unwrap_or_default(),
        other => extract_candidate_list(other),
    }
}

fn extract_candidate_list(expr: Expr) -> Vec<Expr> {
    match expr {
        Expr::List(items) | Expr::Vector(items) => items,
        Expr::Nil => Vec::new(),
        candidate => vec![candidate],
    }
}

fn breakdown_expr(entries: Vec<(String, f32)>) -> Expr {
    Expr::Map(
        entries
            .into_iter()
            .map(|(name, score)| (Expr::Symbol(Symbol::new(name)), number_expr_from_f32(score)))
            .collect(),
    )
}

fn number_expr(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn number_expr_from_f32(value: f32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}
