use sim_kernel::{
    CapabilityName, Cx, Error, Expr, ObjectEncoding, Result, ShapeRef, Symbol, Value,
};
use sim_shape::{Shape, parse_shape_expr, shape_value_with_encoding};

pub(crate) fn expr_to_value(cx: &mut Cx, expr: &Expr) -> Result<Value> {
    match expr {
        Expr::Nil => cx.factory().nil(),
        Expr::Bool(value) => cx.factory().bool(*value),
        Expr::Number(number) => cx
            .factory()
            .number_literal(number.domain.clone(), number.canonical.clone()),
        Expr::Symbol(symbol) => cx.factory().symbol(symbol.clone()),
        Expr::String(text) => cx.factory().string(text.clone()),
        Expr::Bytes(bytes) => cx.factory().bytes(bytes.clone()),
        Expr::List(items) | Expr::Vector(items) => {
            let values = items
                .iter()
                .map(|item| expr_to_value(cx, item))
                .collect::<Result<Vec<_>>>()?;
            cx.factory().list(values)
        }
        Expr::Map(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| {
                    let Expr::Symbol(key) = key else {
                        return Err(Error::TypeMismatch {
                            expected: "symbol table key",
                            found: "non-symbol",
                        });
                    };
                    Ok((key.clone(), expr_to_value(cx, value)?))
                })
                .collect::<Result<Vec<_>>>()?;
            cx.factory().table(values)
        }
        _ => cx.factory().expr(expr.clone()),
    }
}

pub(crate) fn parse_capabilities_expr(expr: &Expr) -> Result<Vec<CapabilityName>> {
    match expr {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => items.iter().map(capability_from_expr).collect(),
        Expr::Symbol(_) | Expr::String(_) => Ok(vec![capability_from_expr(expr)?]),
        _ => Err(Error::TypeMismatch {
            expected: "capability list",
            found: "non-list",
        }),
    }
}

pub(crate) fn capability_from_expr(expr: &Expr) -> Result<CapabilityName> {
    match expr {
        Expr::Symbol(symbol) => Ok(CapabilityName::new(symbol.to_string())),
        Expr::String(text) => Ok(CapabilityName::new(text.clone())),
        _ => Err(Error::TypeMismatch {
            expected: "capability symbol or string",
            found: "non-capability",
        }),
    }
}

pub(crate) fn shape_from_expr(
    _cx: &mut Cx,
    expr: &Expr,
    tool_name: &Symbol,
    slot: &str,
) -> Result<ShapeRef> {
    let shape = parse_shape_expr(literal_expr(expr))?;
    let symbol = Symbol::qualified(tool_name.to_string(), format!("{slot}-shape"));
    Ok(shape_value_with_encoding(
        symbol.clone(),
        shape,
        ObjectEncoding::Constructor {
            class: Symbol::qualified("core", "ExactExprShape"),
            args: vec![literal_expr(expr).clone()],
        },
    ))
}

pub(crate) fn shape_protocol(shape: &ShapeRef) -> Result<&dyn Shape> {
    shape.object().as_shape().ok_or(Error::TypeMismatch {
        expected: "shape",
        found: "non-shape",
    })
}

pub(crate) fn shape_id(shape: &ShapeRef) -> sim_kernel::ShapeId {
    shape_protocol(shape)
        .ok()
        .and_then(|shape| shape.id())
        .unwrap_or(sim_kernel::ShapeId(0))
}

pub(crate) fn installed_codecs(cx: &Cx) -> Vec<Symbol> {
    cx.registry().codecs().keys().cloned().collect()
}

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

pub(crate) fn literal_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Quote { expr, .. } => expr,
        _ => expr,
    }
}

pub(crate) fn symbol_of(expr: &Expr, message: &'static str) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn string_literal(expr: &Expr, message: &'static str) -> Result<String> {
    match literal_expr(expr) {
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn string_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<String> {
    match value.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn stringish_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<String> {
    match value.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn u32_from_expr(expr: &Expr, message: &'static str) -> Result<u32> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse::<u32>()
            .map_err(|_| Error::Eval(message.to_owned())),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn u32_from_value(cx: &mut Cx, value: Value, message: &'static str) -> Result<u32> {
    u32_from_expr(&value.object().as_expr(cx)?, message)
}

pub(crate) fn symbol_from_eval(cx: &mut Cx, expr: &Expr, message: &'static str) -> Result<Symbol> {
    let value = cx.eval_expr(expr.clone())?;
    symbol_from_value(cx, value, message)
}

pub(crate) fn symbol_from_value(
    cx: &mut Cx,
    value: Value,
    message: &'static str,
) -> Result<Symbol> {
    match value.object().as_expr(cx)? {
        Expr::Symbol(symbol) => Ok(symbol),
        Expr::String(text) => Ok(Symbol::new(text)),
        _ => Err(Error::Eval(message.to_owned())),
    }
}

pub(crate) fn f64_from_value(cx: &mut Cx, value: Value, message: &'static str) -> Result<f64> {
    match value.object().as_expr(cx)? {
        Expr::Number(number) => number
            .canonical
            .parse::<f64>()
            .map_err(|_| Error::Eval(message.to_owned())),
        _ => Err(Error::Eval(message.to_owned())),
    }
}
