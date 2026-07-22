use super::query::decode_query;
use crate::require_net_http_capability;
use sim_kernel::{Cx, Error, Expr, Result};
#[cfg(feature = "agent-net")]
use sim_kernel::{NumberLiteral, Symbol};
#[cfg(feature = "agent-net")]
use std::time::Duration;

#[cfg(feature = "agent-net")]
const DEFAULT_WEB_TIMEOUT_MS: u64 = 250;
#[cfg(feature = "agent-net")]
const MAX_WEB_BODY_BYTES: usize = 16 * 1024;

pub(super) fn web_result_expr(cx: &Cx, endpoint: &str, expr: Expr) -> Result<Expr> {
    require_net_http_capability(cx)?;
    let (query_expr, _limit) = decode_query(expr)?;
    let query = query_text(&query_expr)?;
    let url = resolve_url(endpoint, &query);
    #[cfg(feature = "agent-net")]
    {
        let response = sim_lib_server::http_get(
            &url,
            Duration::from_millis(DEFAULT_WEB_TIMEOUT_MS),
            MAX_WEB_BODY_BYTES,
        )?;
        Ok(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("status")),
                Expr::Number(NumberLiteral {
                    domain: Symbol::qualified("numbers", "i64"),
                    canonical: response.status.to_string(),
                }),
            ),
            (Expr::Symbol(Symbol::new("url")), Expr::String(response.url)),
            (
                Expr::Symbol(Symbol::new("body")),
                Expr::String(String::from_utf8_lossy(&response.body).into_owned()),
            ),
        ]))
    }
    #[cfg(not(feature = "agent-net"))]
    {
        let _ = url;
        Err(Error::Eval("web retriever requires agent-net".to_owned()))
    }
}

fn query_text(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval(
            "retriever/web expects a string or symbol query".to_owned(),
        )),
    }
}

fn resolve_url(endpoint: &str, query: &str) -> String {
    if query.starts_with("http://") {
        return query.to_owned();
    }
    if endpoint.ends_with('/') {
        format!("{endpoint}{query}")
    } else {
        format!("{endpoint}/{query}")
    }
}
