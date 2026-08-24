use super::query::decode_query;
use crate::require_net_http_capability;
use sim_kernel::{Cx, Error, Expr, Result};
#[cfg(feature = "agent-net")]
use sim_kernel::{NumberLiteral, Symbol};
#[cfg(feature = "agent-net")]
use std::sync::{Arc, OnceLock};

pub(super) fn web_result_expr(cx: &mut Cx, endpoint: &str, expr: Expr) -> Result<Expr> {
    require_net_http_capability(cx)?;
    let (query_expr, _limit) = decode_query(expr)?;
    let query = query_text(&query_expr)?;
    let url = resolve_url(endpoint, &query);
    #[cfg(feature = "agent-net")]
    {
        use sim_lib_web_fetch::{
            FetchMode, FetchPlan, MemoryCaptureDir, NetHttpExecutor, PublicWebEgress, WebFetcher,
        };
        static STORE: OnceLock<Arc<MemoryCaptureDir>> = OnceLock::new();
        let store = STORE
            .get_or_init(|| Arc::new(MemoryCaptureDir::default()))
            .clone();
        let mut policy = sim_lib_net_http::Policy::default();
        policy.max_response_bytes = 16 * 1024;
        policy.total_timeout = std::time::Duration::from_millis(250);
        let fetcher = WebFetcher::new(
            Arc::new(NetHttpExecutor::new(sim_lib_net_http::TcpConnector, policy)),
            store,
            Arc::new(PublicWebEgress),
        );
        let response = fetcher
            .capture(cx, FetchPlan::get(&url, FetchMode::PreferCache))
            .map_err(|e| Error::Eval(e.to_string()))?;
        Ok(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("status")),
                Expr::Number(NumberLiteral {
                    domain: Symbol::qualified("numbers", "i64"),
                    canonical: response.capture.exchange.status.to_string(),
                }),
            ),
            (
                Expr::Symbol(Symbol::new("url")),
                Expr::String(response.capture.exchange.final_uri),
            ),
            (
                Expr::Symbol(Symbol::new("body")),
                Expr::String(String::from_utf8_lossy(&response.capture.body).into_owned()),
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
