use sim_codec::{Input, encode_with_codec};
use sim_kernel::{
    Cx, EncodeOptions, Expr, ReadPolicy, Result, Symbol, Value, eval_fabric_capability,
};

use crate::{Connection, ensure_installed_codec};

use super::spec::DriverSpec;

/// Where a REPL run sends its rendered result.
#[derive(Clone)]
pub enum ReplOutput {
    /// Write the result through the configured line driver.
    Driver,
    /// Return the rendered result as a string value.
    String,
}

/// Configuration for a single [`run_repl`] invocation.
pub struct ReplOptions {
    /// Connection that evaluates each line.
    pub connection: Connection,
    /// Codec for decoding input and encoding rendered results.
    pub codec: Symbol,
    /// Prompt string shown before each interactive read.
    pub prompt: String,
    /// Driver that supplies input lines and receives output.
    pub driver: DriverSpec,
    /// Optional single input to evaluate once instead of looping.
    pub input: Option<String>,
    /// Where to send the rendered result.
    pub output: ReplOutput,
}

/// Runs a REPL against `options.connection`: either evaluates a single supplied
/// input and returns its rendered result, or loops over driver-supplied lines,
/// decoding, evaluating, and writing each result until input is exhausted.
pub fn run_repl(cx: &mut Cx, options: ReplOptions) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    for capability in options.driver.required_capabilities() {
        cx.require(&capability)?;
    }
    ensure_installed_codec(cx, &options.codec)?;
    if let Some(input) = options.input {
        let rendered = run_once(cx, &options.connection, &options.codec, &input)?;
        return match options.output {
            ReplOutput::String => cx.factory().string(rendered),
            ReplOutput::Driver => cx.factory().nil(),
        };
    }

    let mut driver = options.driver.create_driver(cx)?;
    while let Some(input) = driver.read_line(cx, &options.prompt)? {
        if input.trim().is_empty() {
            continue;
        }
        match run_once(cx, &options.connection, &options.codec, &input) {
            Ok(rendered) => {
                driver.write_output(cx, &(rendered + "\n"))?;
            }
            Err(err) => {
                driver.write_output(cx, &format!("error: {err}\n"))?;
            }
        }
    }
    cx.factory().nil()
}

fn run_once(cx: &mut Cx, connection: &Connection, codec: &Symbol, input: &str) -> Result<String> {
    let expr = lower_repl_expr(sim_codec::decode_with_codec(
        cx,
        codec,
        Input::Text(input.to_owned()),
        ReadPolicy::default(),
    )?);
    let value = connection.request(cx, expr, None, Vec::new())?;
    render_value(cx, codec, &value)
}

pub(crate) fn lower_repl_expr(expr: Expr) -> Expr {
    match expr {
        Expr::List(items) => {
            let mut items = items.into_iter();
            let Some(operator) = items.next() else {
                return Expr::List(Vec::new());
            };
            Expr::Call {
                operator: Box::new(lower_repl_operator(operator)),
                args: items.map(lower_repl_expr).collect(),
            }
        }
        Expr::Vector(items) => Expr::Vector(items.into_iter().map(lower_repl_expr).collect()),
        Expr::Map(entries) => Expr::Map(
            entries
                .into_iter()
                .map(|(key, value)| (key, lower_repl_expr(value)))
                .collect(),
        ),
        Expr::Set(items) => Expr::Set(items.into_iter().map(lower_repl_expr).collect()),
        Expr::Block(items) => Expr::Block(items.into_iter().map(lower_repl_expr).collect()),
        Expr::Annotated { expr, annotations } => Expr::Annotated {
            expr: Box::new(lower_repl_expr(*expr)),
            annotations,
        },
        other => other,
    }
}

fn lower_repl_operator(expr: Expr) -> Expr {
    match expr {
        Expr::Symbol(symbol) => Expr::Symbol(match symbol.name.as_ref() {
            "+" if symbol.namespace.is_none() => Symbol::qualified("math", "add"),
            "-" if symbol.namespace.is_none() => Symbol::qualified("math", "sub"),
            "*" if symbol.namespace.is_none() => Symbol::qualified("math", "mul"),
            "/" if symbol.namespace.is_none() => Symbol::qualified("math", "div"),
            _ => symbol,
        }),
        other => lower_repl_expr(other),
    }
}

fn render_value(cx: &mut Cx, codec: &Symbol, value: &Value) -> Result<String> {
    let expr = value.object().as_expr(cx)?;
    encode_with_codec(cx, codec, &expr, EncodeOptions::default())?.into_text()
}
