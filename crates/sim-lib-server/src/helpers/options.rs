use std::time::Duration;

use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};

use super::{capability_names_from_value, ensure_installed_codec, symbol_from_value};

pub(crate) fn keyword(expr: &Expr) -> Result<String> {
    let Expr::Symbol(symbol) = expr else {
        return Err(Error::TypeMismatch {
            expected: "keyword symbol",
            found: "non-symbol",
        });
    };
    let Some(keyword) = symbol.name.strip_prefix(':') else {
        return Err(Error::Eval(format!(
            "expected keyword option, found {symbol}"
        )));
    };
    Ok(keyword.to_owned())
}

pub(crate) fn symbol_of(expr: &Expr, message: &'static str) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn literal_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Quote { expr, .. } => expr,
        _ => expr,
    }
}

pub(crate) fn parse_server_options<F>(
    cx: &mut Cx,
    options: &[Expr],
    name: &str,
    mut f: F,
) -> Result<()>
where
    F: FnMut(&mut Cx, &str, &Expr) -> Result<()>,
{
    if !options.len().is_multiple_of(2) {
        return Err(Error::Eval(format!(
            "{name} options must be key/value pairs"
        )));
    }
    for pair in options.chunks(2) {
        let key = keyword(&pair[0])?;
        f(cx, key.as_str(), &pair[1])?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_message_options(
    cx: &mut Cx,
    options: &[Expr],
    name: &str,
    codec: &mut Symbol,
    deadline: &mut Option<Duration>,
    required_capabilities: &mut Vec<CapabilityName>,
    reply_codec_hint: Option<&mut Option<Symbol>>,
    consistency: Option<&mut sim_kernel::Consistency>,
) -> Result<()> {
    if !options.len().is_multiple_of(2) {
        return Err(Error::Eval(format!(
            "{name} options must be key/value pairs"
        )));
    }
    let mut reply_codec_hint = reply_codec_hint;
    let mut consistency = consistency;
    for pair in options.chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "codec" => {
                let value = cx.eval_expr(pair[1].clone())?;
                *codec = symbol_from_value(cx, value, "message :codec expects a symbol")?;
                ensure_installed_codec(cx, codec)?;
            }
            "deadline" | "timeout" => {
                let value = cx.eval_expr(pair[1].clone())?;
                *deadline = Some(parse_duration_value(cx, value)?)
            }
            "requires" => {
                let value = cx.eval_expr(pair[1].clone())?;
                *required_capabilities = capability_names_from_value(cx, value)?;
            }
            "reply-codec" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let hint = symbol_from_value(cx, value, "message :reply-codec expects a symbol")?;
                ensure_installed_codec(cx, &hint)?;
                let Some(slot) = reply_codec_hint.as_deref_mut() else {
                    return Err(Error::Eval(format!("{name} does not support :reply-codec")));
                };
                *slot = Some(hint);
            }
            "consistency" => {
                let value = cx.eval_expr(pair[1].clone())?;
                let parsed = parse_consistency_value(cx, value)?;
                let Some(slot) = consistency.as_deref_mut() else {
                    return Err(Error::Eval(format!("{name} does not support :consistency")));
                };
                *slot = parsed;
            }
            other => return Err(Error::Eval(format!("{name}: unknown option :{other}"))),
        }
    }
    Ok(())
}

pub(crate) fn parse_duration_value(cx: &mut Cx, value: sim_kernel::Value) -> Result<Duration> {
    parse_duration(&value.object().as_expr(cx)?)
}

pub(crate) fn usize_from_value(
    cx: &mut Cx,
    value: sim_kernel::Value,
    message: &'static str,
) -> Result<usize> {
    match value.object().as_expr(cx)? {
        Expr::String(text) => text
            .parse::<usize>()
            .map_err(|_| Error::Eval(message.to_owned())),
        Expr::Number(number) => number
            .canonical
            .parse::<usize>()
            .map_err(|_| Error::Eval(message.to_owned())),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

/// Parses a [`Duration`] from a duration string or an integer millisecond
/// count.
pub fn parse_duration(expr: &Expr) -> Result<Duration> {
    match expr {
        Expr::String(text) => parse_duration_text(text),
        Expr::Number(number) => {
            let millis = number.canonical.parse::<u64>().map_err(|_| {
                Error::Eval(format!(
                    "deadline {} is not an integer millisecond count",
                    number.canonical
                ))
            })?;
            Ok(Duration::from_millis(millis))
        }
        _ => Err(Error::TypeMismatch {
            expected: "deadline string or integer number",
            found: "non-deadline",
        }),
    }
}

pub(crate) fn parse_optional_duration(expr: &Expr) -> Result<Option<Duration>> {
    match expr {
        Expr::Nil => Ok(None),
        _ => parse_duration(expr).map(Some),
    }
}

pub(crate) fn format_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 && duration.as_secs() > 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn parse_duration_text(text: &str) -> Result<Duration> {
    let (number, unit) = if let Some(number) = text.strip_suffix("ms") {
        (number, "ms")
    } else if let Some(number) = text.strip_suffix('s') {
        (number, "s")
    } else if let Some(number) = text.strip_suffix('m') {
        (number, "m")
    } else if let Some(number) = text.strip_suffix('h') {
        (number, "h")
    } else {
        return Err(Error::Eval(format!(
            "deadline {text} must end with ms, s, m, or h"
        )));
    };

    let value = number
        .parse::<u64>()
        .map_err(|_| Error::Eval(format!("deadline {text} has an invalid numeric prefix")))?;
    Ok(match unit {
        "ms" => Duration::from_millis(value),
        "s" => Duration::from_secs(value),
        "m" => Duration::from_secs(value.saturating_mul(60)),
        "h" => Duration::from_secs(value.saturating_mul(60 * 60)),
        _ => unreachable!(),
    })
}

pub(crate) fn parse_consistency_value(
    cx: &mut Cx,
    value: sim_kernel::Value,
) -> Result<sim_kernel::Consistency> {
    let name = match value.object().as_expr(cx)? {
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text,
        _ => {
            return Err(Error::TypeMismatch {
                expected: "consistency symbol or string",
                found: "non-consistency",
            });
        }
    };
    match name.as_str() {
        "local-only" => Ok(sim_kernel::Consistency::LocalOnly),
        "local-first" => Ok(sim_kernel::Consistency::LocalFirst),
        "remote-only" => Ok(sim_kernel::Consistency::RemoteOnly),
        _ => Err(Error::Eval(format!(
            "unsupported realize consistency {name}"
        ))),
    }
}
