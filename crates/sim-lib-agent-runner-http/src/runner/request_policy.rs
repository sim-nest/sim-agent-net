use super::*;

pub(super) fn request_privacy_no_raw(request: &ModelRequest) -> bool {
    request
        .extra
        .iter()
        .find_map(|(key, value)| is_field(key, "privacy").then_some(value))
        .is_some_and(privacy_expr_no_raw)
}

fn privacy_expr_no_raw(expr: &Expr) -> bool {
    match expr {
        Expr::Symbol(symbol) => symbol.name.as_ref() == "no-raw",
        Expr::String(text) => text == "no-raw",
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) => {
            items.iter().any(privacy_expr_no_raw)
        }
        Expr::Map(entries) => entries.iter().any(|(key, value)| {
            is_field(key, "no-raw") && !matches!(value, Expr::Bool(false) | Expr::Nil)
        }),
        _ => false,
    }
}

fn is_field(expr: &Expr, name: &str) -> bool {
    matches!(
        expr,
        Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name
    )
}

pub(super) fn extra_field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    entries.iter().find_map(|(key, value)| {
        if is_field(key, name) {
            Some(value)
        } else {
            None
        }
    })
}

fn extra_field_mut<'a>(entries: &'a mut [(Expr, Expr)], name: &str) -> Option<&'a mut Expr> {
    entries.iter_mut().find_map(|(key, value)| {
        if is_field(key, name) {
            Some(value)
        } else {
            None
        }
    })
}

pub(super) fn extra_symbol(entries: &[(Expr, Expr)], name: &str) -> Option<Symbol> {
    match extra_field(entries, name) {
        Some(Expr::Symbol(symbol)) => Some(symbol.clone()),
        _ => None,
    }
}

pub(super) fn upsert_extra(entries: &mut Vec<(Expr, Expr)>, name: &str, value: Expr) {
    if let Some((_, existing)) = entries.iter_mut().find(|(key, _)| is_field(key, name)) {
        *existing = value;
        return;
    }
    entries.push((Expr::Symbol(Symbol::new(name)), value));
}

pub(super) fn strip_output_grammar(entries: &mut Vec<(Expr, Expr)>) {
    entries.retain(|(key, _)| {
        !is_field(key, OUTPUT_GRAMMAR_EXTRA)
            && !is_field(key, OUTPUT_GRAMMAR_DIALECT_EXTRA)
            && !is_field(key, OUTPUT_GRAMMAR_REQUIRED_EXTRA)
            && !is_field(key, RETURN_SHAPE_EXTRA)
    });
}

pub(super) fn remove_extra(entries: &mut Vec<(Expr, Expr)>, name: &str) {
    entries.retain(|(key, _)| !is_field(key, name));
}

pub(super) fn normalize_return_shape_for_output_grammar(entries: &mut [(Expr, Expr)]) {
    let Some(shape_expr) = extra_field_mut(entries, RETURN_SHAPE_EXTRA) else {
        return;
    };
    let Expr::Symbol(symbol) = shape_expr else {
        return;
    };
    if symbol.namespace.as_deref() != Some("core") {
        return;
    }
    if matches!(
        symbol.name.as_ref(),
        "Any" | "Bool" | "List" | "Map" | "Nil" | "Number" | "String" | "Symbol"
    ) {
        *shape_expr = Expr::Symbol(Symbol::new(symbol.name.to_string()));
    }
}

pub(super) fn explicit_output_grammar_matches(
    entries: &[(Expr, Expr)],
    dialect: GrammarDialect,
) -> bool {
    matches!(
        extra_field(entries, OUTPUT_GRAMMAR_EXTRA),
        Some(Expr::String(_))
    ) && extra_field(entries, OUTPUT_GRAMMAR_DIALECT_EXTRA)
        .and_then(|expr| match expr {
            Expr::Symbol(symbol) => grammar_dialect_from_symbol_local(symbol),
            _ => None,
        })
        .unwrap_or(GrammarDialect::JsonSchema)
        == dialect
}

fn grammar_dialect_from_symbol_local(symbol: &Symbol) -> Option<GrammarDialect> {
    match symbol.name.as_ref() {
        "json-schema" if symbol.namespace.is_none() => Some(GrammarDialect::JsonSchema),
        "gbnf" if symbol.namespace.is_none() => Some(GrammarDialect::Gbnf),
        "sexpr" if symbol.namespace.is_none() => Some(GrammarDialect::SExpr),
        _ => None,
    }
}
