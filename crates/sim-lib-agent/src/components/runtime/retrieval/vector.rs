use super::query::decode_query;
use crate::memory::{SharedEntries, shared_blackboard_entries};
use crate::{cosine, embed, lock_entries};
use sim_kernel::{Cx, Expr, NumberLiteral, Result, Symbol};
use std::{cmp::Ordering, collections::HashSet};

pub(super) fn vector_result_expr(
    _cx: &mut Cx,
    store: &str,
    corpus: &[String],
    expr: Expr,
) -> Result<Expr> {
    let (query_expr, limit) = decode_query(expr)?;
    let query_text = flatten_query_text(&query_expr);
    let entries = vector_entries(store, corpus)?;
    let query_embedding = embed(&query_text);
    let mut scored = Vec::new();
    for entry in lock_entries(&entries, "vector store entries")?.iter() {
        let text = flatten_query_text(entry);
        if text.is_empty() {
            continue;
        }
        let score = cosine(&query_embedding, &embed(&text));
        scored.push((score, text));
    }
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.cmp(&right.1))
    });
    Ok(Expr::List(
        scored
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|(score, text)| {
                Expr::Map(vec![
                    (
                        Expr::Symbol(Symbol::new("score")),
                        Expr::Number(NumberLiteral {
                            domain: Symbol::qualified("numbers", "f64"),
                            canonical: format!("{score:.6}"),
                        }),
                    ),
                    (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
                ])
            })
            .collect(),
    ))
}

fn vector_entries(store: &str, corpus: &[String]) -> Result<SharedEntries> {
    let entries = shared_blackboard_entries(store);
    if corpus.is_empty() {
        return Ok(entries);
    }
    let mut docs = lock_entries(&entries, "vector store entries")?;
    let existing = docs.iter().map(flatten_query_text).collect::<HashSet<_>>();
    for text in corpus {
        if existing.contains(text) {
            continue;
        }
        docs.push(Expr::String(text.clone()));
    }
    drop(docs);
    Ok(entries)
}

fn flatten_query_text(expr: &Expr) -> String {
    match expr {
        Expr::String(text) => text.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::Local(symbol) => symbol.to_string(),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => items
            .iter()
            .map(flatten_query_text)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Expr::Map(entries) => entries
            .iter()
            .flat_map(|(key, value)| [flatten_query_text(key), flatten_query_text(value)])
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Bool(value) => value.to_string(),
        Expr::Bytes(bytes) => format!("{bytes:?}"),
        Expr::Nil => String::new(),
        Expr::Call { operator, args } => {
            let mut parts = vec![flatten_query_text(operator)];
            parts.extend(args.iter().map(flatten_query_text));
            parts
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => [
            operator.to_string(),
            flatten_query_text(left),
            flatten_query_text(right),
        ]
        .into_iter()
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(" "),
        Expr::Prefix { operator, arg } | Expr::Postfix { operator, arg } => {
            [operator.to_string(), flatten_query_text(arg)]
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        Expr::Quote { expr, .. } => flatten_query_text(expr),
        Expr::Extension { tag, payload } => [tag.to_string(), flatten_query_text(payload)]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Expr::Annotated { expr, annotations } => {
            let mut parts = vec![flatten_query_text(expr)];
            for (key, value) in annotations {
                parts.push(key.to_string());
                parts.push(flatten_query_text(value));
            }
            parts
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}
