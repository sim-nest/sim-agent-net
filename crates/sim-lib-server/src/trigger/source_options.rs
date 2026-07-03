use sim_kernel::{Error, Expr, Result};

pub(super) fn source_loopback(expr: &Expr) -> Result<bool> {
    match find_expr(expr, "loopback")? {
        Some(Expr::Bool(value)) => Ok(*value),
        Some(Expr::Nil) | None => Ok(false),
        Some(_) => Err(Error::Eval(
            "trigger source :loopback expects a bool".to_owned(),
        )),
    }
}

#[cfg(any(
    feature = "trigger-webhook",
    feature = "trigger-imap",
    feature = "trigger-smtp",
    feature = "trigger-telegram",
    feature = "trigger-matrix"
))]
pub(super) fn source_string(expr: &Expr, key: &str) -> Result<Option<String>> {
    find_expr(expr, key)?
        .map(|value| match value {
            Expr::String(text) => Ok(text.clone()),
            Expr::Symbol(symbol) => Ok(symbol.to_string()),
            _ => Err(Error::Eval(format!(
                "trigger source :{key} expects a string"
            ))),
        })
        .transpose()
}

#[cfg(any(
    feature = "trigger-webhook",
    feature = "trigger-imap",
    feature = "trigger-smtp"
))]
pub(super) fn source_u16(expr: &Expr, key: &str) -> Result<Option<u16>> {
    find_expr(expr, key)?
        .map(|value| match value {
            Expr::String(text) => text
                .parse::<u16>()
                .map_err(|_| Error::Eval(format!("trigger source :{key} has invalid port"))),
            Expr::Number(number) => number
                .canonical
                .parse::<u16>()
                .map_err(|_| Error::Eval(format!("trigger source :{key} has invalid port"))),
            _ => Err(Error::Eval(format!(
                "trigger source :{key} expects an integer"
            ))),
        })
        .transpose()
}

fn find_expr<'a>(expr: &'a Expr, key: &str) -> Result<Option<&'a Expr>> {
    let items = match expr {
        Expr::Quote { expr, .. } => match expr.as_ref() {
            Expr::List(items) | Expr::Vector(items) => items.as_slice(),
            _ => return Ok(None),
        },
        Expr::List(items) | Expr::Vector(items) => items.as_slice(),
        _ => return Ok(None),
    };
    if items.len() <= 1 {
        return Ok(None);
    }
    if !(items.len() - 1).is_multiple_of(2) {
        return Err(Error::Eval(
            "trigger source options must be key/value pairs".to_owned(),
        ));
    }
    for pair in items[1..].chunks(2) {
        let Expr::Symbol(symbol) = &pair[0] else {
            return Err(Error::TypeMismatch {
                expected: "keyword symbol",
                found: "non-symbol",
            });
        };
        let name = symbol
            .name
            .strip_prefix(':')
            .unwrap_or(symbol.name.as_ref());
        if name == key {
            return Ok(Some(&pair[1]));
        }
    }
    Ok(None)
}
