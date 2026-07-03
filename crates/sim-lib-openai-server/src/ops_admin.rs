use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};

use crate::{
    capabilities::openai_gateway_admin_capability,
    routes::{admin::run_get_expr, errors::OpenAiRouteError},
    server::{GatewayRouteState, GatewayRoutes, GatewayRoutesValue, configure_routes},
};

pub(crate) fn call_admin_state_expr(
    cx: &mut Cx,
    symbol: Symbol,
    args: Vec<Value>,
    report: impl FnOnce(&GatewayRouteState) -> std::result::Result<Expr, OpenAiRouteError>,
) -> Result<Value> {
    cx.require(&openai_gateway_admin_capability())?;
    let routes = routes_arg_or_default(cx, symbol, args)?;
    admin_expr_value(cx, report(routes.state()))
}

pub(crate) fn call_run_get(cx: &mut Cx, symbol: Symbol, args: Vec<Value>) -> Result<Value> {
    cx.require(&openai_gateway_admin_capability())?;
    let mut args = args.into_iter();
    let (routes, run_id) = match (args.next(), args.next(), args.next()) {
        (Some(run_id), None, None) => (configure_routes(), value_string(cx, run_id)?),
        (Some(routes), Some(run_id), None) => {
            (routes_from_value(cx, routes)?, value_string(cx, run_id)?)
        }
        (_, _, _) => {
            return Err(Error::Eval(format!(
                "{symbol} expects a run id, or routes and a run id"
            )));
        }
    };
    admin_expr_value(cx, run_get_expr(routes.state(), &run_id))
}

fn routes_arg_or_default(cx: &mut Cx, symbol: Symbol, args: Vec<Value>) -> Result<GatewayRoutes> {
    match args.len() {
        0 => Ok(configure_routes()),
        1 => routes_from_value(cx, args.into_iter().next().expect("one arg checked")),
        count => Err(Error::Eval(format!(
            "{symbol} expects 0 or 1 argument(s), found {count}"
        ))),
    }
}

fn routes_from_value(cx: &mut Cx, value: Value) -> Result<GatewayRoutes> {
    value
        .object()
        .downcast_ref::<GatewayRoutesValue>()
        .map(|routes| routes.routes().clone())
        .ok_or_else(|| {
            let found = value
                .object()
                .display(cx)
                .unwrap_or_else(|_| "value".to_owned());
            Error::Eval(format!("expected openai gateway routes, found {found}"))
        })
}

fn admin_expr_value(
    cx: &mut Cx,
    expr: std::result::Result<Expr, OpenAiRouteError>,
) -> Result<Value> {
    match expr {
        Ok(expr) => cx.factory().expr(expr),
        Err(error) => Err(Error::Eval(
            String::from_utf8_lossy(error.into_response().body()).into_owned(),
        )),
    }
}

fn value_string(cx: &mut Cx, value: Value) -> Result<String> {
    match value.object().as_expr(cx)? {
        Expr::String(value) => Ok(value),
        _ => Err(Error::Eval("expected string argument".to_owned())),
    }
}
