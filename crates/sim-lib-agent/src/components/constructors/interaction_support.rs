use super::super::{
    model::{JudgeConfig, JudgeCriterion, number_expr_from_f64},
    options::{expr_option, strings_option},
};
use crate::util::stringish_from_value;
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use std::collections::HashMap;

pub(super) fn judge_config_from_options(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
) -> Result<JudgeConfig> {
    let rubric = expr_option(cx, options, "rubric")?.unwrap_or_else(default_judge_rubric);
    let criteria = parse_weighted_criteria(&rubric)?;
    let reference = option_stringish(cx, options, "reference")?;
    let style_markers = strings_option(cx, options, "style-markers")?;
    Ok(JudgeConfig {
        rubric,
        criteria,
        reference,
        style_markers,
    })
}

pub(super) fn judge_spec(config: &JudgeConfig) -> Vec<(Symbol, Expr)> {
    let mut spec = vec![(Symbol::new("rubric"), config.rubric.clone())];
    if let Some(reference) = &config.reference {
        spec.push((Symbol::new("reference"), Expr::String(reference.clone())));
    }
    if !config.style_markers.is_empty() {
        spec.push((
            Symbol::new("style-markers"),
            Expr::List(
                config
                    .style_markers
                    .iter()
                    .cloned()
                    .map(Expr::String)
                    .collect(),
            ),
        ));
    }
    spec
}

pub(super) fn style_word_limit(voice: &str) -> usize {
    match voice {
        "terse" => 12,
        "brief" => 20,
        _ => 32,
    }
}

pub(super) fn glossary_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<Vec<(String, String)>>> {
    let Some(expr) = expr_option(cx, options, key)? else {
        return Ok(None);
    };
    parse_pair_table(&expr).map(Some)
}

pub(super) fn option_stringish(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<String>> {
    options
        .get(key)
        .map(|value| stringish_from_value(cx, value.clone(), "expected string option"))
        .transpose()
}

pub(super) fn pair_table_expr(entries: &[(String, String)]) -> Expr {
    Expr::Map(
        entries
            .iter()
            .map(|(key, value)| {
                (
                    Expr::Symbol(Symbol::new(key.clone())),
                    Expr::String(value.clone()),
                )
            })
            .collect(),
    )
}

pub(super) fn default_glossary(language: &str) -> Vec<(String, String)> {
    match language {
        "es" => vec![
            ("hello".to_owned(), "hola".to_owned()),
            ("world".to_owned(), "mundo".to_owned()),
            ("agent".to_owned(), "agente".to_owned()),
            ("sim".to_owned(), "sim".to_owned()),
        ],
        "sv" => vec![
            ("hello".to_owned(), "hej".to_owned()),
            ("world".to_owned(), "varld".to_owned()),
            ("agent".to_owned(), "agent".to_owned()),
            ("sim".to_owned(), "sim".to_owned()),
        ],
        _ => Vec::new(),
    }
}

pub(super) fn default_translation_table(from: &str, to: &str) -> Vec<(String, String)> {
    match (from, to) {
        ("en", "sv") => vec![
            ("hello".to_owned(), "hej".to_owned()),
            ("world".to_owned(), "varld".to_owned()),
            ("fast".to_owned(), "snabb".to_owned()),
        ],
        ("en", "es") => vec![
            ("hello".to_owned(), "hola".to_owned()),
            ("world".to_owned(), "mundo".to_owned()),
            ("fast".to_owned(), "rapido".to_owned()),
        ],
        _ => Vec::new(),
    }
}

fn default_judge_rubric() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::new(":correctness")),
        number_expr_from_f64(0.5),
        Expr::Symbol(Symbol::new(":style")),
        number_expr_from_f64(0.2),
        Expr::Symbol(Symbol::new(":brevity")),
        number_expr_from_f64(0.3),
    ])
}

fn parse_weighted_criteria(rubric: &Expr) -> Result<Vec<JudgeCriterion>> {
    let mut criteria = Vec::new();
    match rubric {
        Expr::Map(entries) => {
            for (key, value) in entries {
                criteria.push(JudgeCriterion {
                    name: criterion_name(key)?,
                    weight: criterion_weight(value)?,
                });
            }
        }
        Expr::List(items) | Expr::Vector(items) => {
            if !items.len().is_multiple_of(2) {
                return Err(Error::Eval(
                    "judge rubric expects key/value weight pairs".to_owned(),
                ));
            }
            for pair in items.chunks(2) {
                criteria.push(JudgeCriterion {
                    name: criterion_name(&pair[0])?,
                    weight: criterion_weight(&pair[1])?,
                });
            }
        }
        Expr::String(_) | Expr::Symbol(_) => {
            criteria.push(JudgeCriterion {
                name: criterion_name(rubric)?,
                weight: 1.0,
            });
        }
        Expr::Nil => {}
        _ => {
            return Err(Error::Eval(
                "judge rubric expects a map or key/value list".to_owned(),
            ));
        }
    }
    if criteria.is_empty() {
        return Ok(vec![
            JudgeCriterion {
                name: "correctness".to_owned(),
                weight: 0.5,
            },
            JudgeCriterion {
                name: "style".to_owned(),
                weight: 0.2,
            },
            JudgeCriterion {
                name: "brevity".to_owned(),
                weight: 0.3,
            },
        ]);
    }
    Ok(criteria)
}

fn parse_pair_table(expr: &Expr) -> Result<Vec<(String, String)>> {
    match expr {
        Expr::Map(entries) => entries
            .iter()
            .map(|(key, value)| Ok((criterion_name(key)?, string_expr(value)?)))
            .collect(),
        Expr::List(items) | Expr::Vector(items) => {
            if !items.len().is_multiple_of(2) {
                return Err(Error::Eval(
                    "table option expects key/value pairs".to_owned(),
                ));
            }
            items
                .chunks(2)
                .map(|pair| Ok((criterion_name(&pair[0])?, string_expr(&pair[1])?)))
                .collect()
        }
        Expr::Nil => Ok(Vec::new()),
        _ => Err(Error::Eval(
            "table option expects a map or key/value list".to_owned(),
        )),
    }
}

fn criterion_name(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.name.trim_start_matches(':').to_owned()),
        Expr::String(text) => Ok(text.trim_start_matches(':').to_owned()),
        _ => Err(Error::Eval(
            "judge rubric criterion expects a symbol or string".to_owned(),
        )),
    }
}

fn criterion_weight(expr: &Expr) -> Result<f64> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse::<f64>()
            .map_err(|_| Error::Eval("judge rubric weight must be numeric".to_owned())),
        _ => Err(Error::Eval(
            "judge rubric weight must be numeric".to_owned(),
        )),
    }
}

fn string_expr(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval(
            "table option values must be strings or symbols".to_owned(),
        )),
    }
}
