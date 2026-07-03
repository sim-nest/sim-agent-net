use sim_kernel::{Error, Expr, Result, Symbol};

pub(super) const DEFAULT_QUERY_LIMIT: u32 = 3;

pub(super) fn decode_query(expr: Expr) -> Result<(Expr, u32)> {
    if let Expr::List(items) | Expr::Vector(items) = &expr
        && let [Expr::Symbol(symbol), query, limit] = items.as_slice()
        && is_query_symbol(symbol)
    {
        return Ok((query.clone(), parse_limit(limit)?));
    }
    Ok((expr, DEFAULT_QUERY_LIMIT))
}

fn is_query_symbol(symbol: &Symbol) -> bool {
    matches!(symbol.name.as_ref(), "query" | "retriever/query")
}

fn parse_limit(expr: &Expr) -> Result<u32> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse::<u32>()
            .map_err(|_| Error::Eval("retriever query count must be an integer".to_owned())),
        _ => Err(Error::Eval(
            "retriever query count must be an integer".to_owned(),
        )),
    }
}
