use super::super::store::snapshot_entries;
use super::types::MemoryBackend;
use sim_kernel::{Cx, Error, Expr, Result, Value};

pub(super) fn answer_memory_request(
    memory: &dyn MemoryBackend,
    cx: &mut Cx,
    expr: Expr,
) -> Result<Value> {
    let (op, args) = memory_request_parts(expr)?;
    match op.as_str() {
        "append" | "memory/append" => {
            let [msg] = args.as_slice() else {
                return Err(Error::Eval(
                    "memory append frame expects one message".to_owned(),
                ));
            };
            let msg = crate::expr_to_value(cx, msg)?;
            memory.append(cx, msg)?;
            cx.factory().nil()
        }
        "recent" | "memory/recent" => {
            let [count] = args.as_slice() else {
                return Err(Error::Eval(
                    "memory recent frame expects one count".to_owned(),
                ));
            };
            let count =
                crate::u32_from_expr(count, "memory recent frame expects an integer count")?;
            let recent = memory.recent(cx, count)?;
            cx.factory().list(recent)
        }
        "scan" | "memory/scan" => {
            let entries = match args.as_slice() {
                [] => snapshot_entries(memory.snapshot(cx)?)?
                    .into_iter()
                    .map(|entry| crate::expr_to_value(cx, &entry))
                    .collect::<Result<Vec<_>>>()?,
                [count] => {
                    let count =
                        crate::u32_from_expr(count, "memory scan frame expects an integer count")?;
                    memory.recent(cx, count)?
                }
                _ => {
                    return Err(Error::Eval(
                        "memory scan frame expects zero or one count".to_owned(),
                    ));
                }
            };
            cx.factory().list(entries)
        }
        "search" | "memory/search" => {
            let [query, k] = args.as_slice() else {
                return Err(Error::Eval(
                    "memory search frame expects a query and count".to_owned(),
                ));
            };
            let k = crate::u32_from_expr(k, "memory search frame expects an integer count")?;
            let results = memory.search(cx, query.clone(), k)?;
            cx.factory().list(results)
        }
        "snapshot" | "memory/snapshot" => {
            let snapshot = memory.snapshot(cx)?;
            cx.factory().expr(snapshot)
        }
        "restore" | "memory/restore" => {
            let [snapshot] = args.as_slice() else {
                return Err(Error::Eval(
                    "memory restore frame expects one snapshot".to_owned(),
                ));
            };
            memory.restore(cx, snapshot.clone())?;
            cx.factory().nil()
        }
        _ => Err(Error::Eval(format!("unknown memory frame operation {op}"))),
    }
}

fn memory_request_parts(expr: Expr) -> Result<(String, Vec<Expr>)> {
    let (Expr::List(items) | Expr::Vector(items)) = expr else {
        return Err(Error::Eval(
            "memory request frames expect a list or vector payload".to_owned(),
        ));
    };
    let Some((head, rest)) = items.split_first() else {
        return Err(Error::Eval(
            "memory request frames cannot be empty".to_owned(),
        ));
    };
    let Expr::Symbol(symbol) = head else {
        return Err(Error::Eval(
            "memory request frames expect a symbol operation".to_owned(),
        ));
    };
    Ok((symbol.to_string(), rest.to_vec()))
}
