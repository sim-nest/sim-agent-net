//! Lisp projections over the canonical conduct and run command surfaces.

use sim_kernel::{Args, Cx, Error, Expr, Result, Value};

pub(crate) fn conduct_command_value(
    cx: &mut Cx,
    group: &str,
    command: &str,
    args: Args,
) -> Result<Value> {
    let mut command_args = vec![group.to_owned(), command.to_owned()];
    for value in args.values() {
        command_args.push(match value.object().as_expr(cx)? {
            Expr::String(text) => text,
            Expr::Symbol(symbol) => symbol.to_string(),
            other => {
                return Err(Error::Eval(format!(
                    "{group}/{command} expects string or symbol arguments, found {other:?}"
                )));
            }
        });
    }
    let result = crate::cli::dispatch_agent_cli(cx, &command_args)?;
    cx.factory().expr(result)
}
