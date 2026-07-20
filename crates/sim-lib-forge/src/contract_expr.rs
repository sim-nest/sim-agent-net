use sim_kernel::{Cx, Error, Expr, Result, ShapeId, Symbol};
use sim_shape::{check_shape_on_expr, parse_shape_expr};
use sim_value::{
    access::{entry_field, map_entries},
    build::entry,
};

use crate::{ContractCard, ContractGap};

impl ContractCard {
    /// Encodes this card as tagged open data accepted by [`contract_card_shape`].
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            entry("kind", Expr::Symbol(contract_card_symbol())),
            entry("lib", Expr::Symbol(self.lib.clone())),
            entry("export-kind", Expr::Symbol(self.export_kind.clone())),
            entry("symbol", Expr::Symbol(self.symbol.clone())),
            entry("args-shape", option_expr(self.args_shape.clone())),
            entry("result-shape", option_expr(self.result_shape.clone())),
            entry(
                "capabilities",
                Expr::List(
                    self.capability_symbols
                        .iter()
                        .cloned()
                        .map(Expr::Symbol)
                        .collect(),
                ),
            ),
            entry("card-requires", option_expr(self.card_requires.clone())),
            entry("summary", Expr::String(self.summary.clone())),
            entry("example", option_expr(self.example.clone())),
            entry(
                "partial",
                Expr::List(
                    self.partial
                        .iter()
                        .map(|gap| Expr::Symbol(gap.as_symbol()))
                        .collect(),
                ),
            ),
        ])
    }
}

impl ContractGap {
    /// Returns the stable symbol used in encoded contract data.
    pub fn as_symbol(&self) -> Symbol {
        match self {
            Self::MissingCallableShape => {
                Symbol::qualified("contract-gap", "missing-callable-shape")
            }
            Self::MissingCard => Symbol::qualified("contract-gap", "missing-card"),
            Self::MissingExample => Symbol::qualified("contract-gap", "missing-example"),
            Self::SynthesizedExample => Symbol::qualified("contract-gap", "synthesized-example"),
        }
    }

    fn from_symbol(symbol: &Symbol) -> Result<Self> {
        let namespace = symbol.namespace.as_deref();
        let name = symbol.name.as_ref();
        match (namespace, name) {
            (Some("contract-gap"), "missing-callable-shape") => Ok(Self::MissingCallableShape),
            (Some("contract-gap"), "missing-card") => Ok(Self::MissingCard),
            (Some("contract-gap"), "missing-example") => Ok(Self::MissingExample),
            (Some("contract-gap"), "synthesized-example") => Ok(Self::SynthesizedExample),
            _ => Err(Error::Eval(format!("unknown contract gap {symbol}"))),
        }
    }
}

/// The Shape a `ContractCard::as_expr` value conforms to.
///
/// Other repos can check tagged deck data against this Shape instead of trusting
/// untyped `Expr` payloads.
pub fn contract_card_shape() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::qualified("shape", "table-open")),
        Expr::List(vec![
            shape_field("kind", symbol_shape()),
            shape_field("lib", symbol_shape()),
            shape_field("export-kind", symbol_shape()),
            shape_field("symbol", symbol_shape()),
            shape_field("args-shape", any_shape()),
            shape_field("result-shape", any_shape()),
            shape_field(
                "capabilities",
                Expr::List(vec![
                    Expr::Symbol(Symbol::qualified("shape", "repeat")),
                    symbol_shape(),
                ]),
            ),
            shape_field("card-requires", any_shape()),
            shape_field("summary", string_shape()),
            shape_field("example", any_shape()),
            shape_field(
                "partial",
                Expr::List(vec![
                    Expr::Symbol(Symbol::qualified("shape", "repeat")),
                    symbol_shape(),
                ]),
            ),
        ]),
    ])
}

/// Decodes a [`ContractCard`] from data that checks against [`contract_card_shape`].
pub fn contract_card_from_expr(cx: &mut Cx, e: &Expr) -> Result<ContractCard> {
    let shape = parse_shape_expr(&contract_card_shape())?;
    let checked = check_shape_on_expr(shape.as_ref(), cx, e)?;
    if !checked.accepted {
        return Err(Error::WrongShape {
            expected: ShapeId(0),
            diagnostics: checked.diagnostics,
        });
    }

    let entries = map_entries(e, "contract card")?;
    let kind = required_symbol(entries, "kind")?;
    if kind != contract_card_symbol() {
        return Err(Error::Eval(format!(
            "contract card kind must be {}, found {kind}",
            contract_card_symbol()
        )));
    }

    Ok(ContractCard {
        lib: required_symbol(entries, "lib")?,
        export_kind: required_symbol(entries, "export-kind")?,
        symbol: required_symbol(entries, "symbol")?,
        args_shape: optional_expr(entries, "args-shape")?,
        result_shape: optional_expr(entries, "result-shape")?,
        capability_symbols: required_symbol_list(entries, "capabilities")?,
        card_requires: optional_expr(entries, "card-requires")?,
        summary: required_string(entries, "summary")?,
        example: optional_expr(entries, "example")?,
        partial: required_symbol_list(entries, "partial")?
            .iter()
            .map(ContractGap::from_symbol)
            .collect::<Result<Vec<_>>>()?,
    })
}

fn option_expr(expr: Option<Expr>) -> Expr {
    expr.unwrap_or(Expr::Nil)
}

fn optional_expr(entries: &[(Expr, Expr)], field: &str) -> Result<Option<Expr>> {
    Ok(match required_expr(entries, field)? {
        Expr::Nil => None,
        expr => Some(expr.clone()),
    })
}

fn required_expr<'a>(entries: &'a [(Expr, Expr)], field: &str) -> Result<&'a Expr> {
    entry_field(entries, field)
        .ok_or_else(|| Error::Eval(format!("contract card is missing field {field}")))
}

fn required_symbol(entries: &[(Expr, Expr)], field: &str) -> Result<Symbol> {
    match required_expr(entries, field)? {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::Eval(format!(
            "contract card field {field} must be a symbol"
        ))),
    }
}

fn required_string(entries: &[(Expr, Expr)], field: &str) -> Result<String> {
    match required_expr(entries, field)? {
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(format!(
            "contract card field {field} must be a string"
        ))),
    }
}

fn required_symbol_list(entries: &[(Expr, Expr)], field: &str) -> Result<Vec<Symbol>> {
    match required_expr(entries, field)? {
        Expr::List(items) => items
            .iter()
            .map(|item| match item {
                Expr::Symbol(symbol) => Ok(symbol.clone()),
                _ => Err(Error::Eval(format!(
                    "contract card field {field} must contain only symbols"
                ))),
            })
            .collect(),
        _ => Err(Error::Eval(format!(
            "contract card field {field} must be a list"
        ))),
    }
}

fn shape_field(field: &str, shape: Expr) -> Expr {
    Expr::List(vec![Expr::Symbol(Symbol::new(field)), shape])
}

fn any_shape() -> Expr {
    Expr::Symbol(Symbol::new("Any"))
}

fn string_shape() -> Expr {
    Expr::Symbol(Symbol::new("String"))
}

fn symbol_shape() -> Expr {
    Expr::Symbol(Symbol::new("Symbol"))
}

fn contract_card_symbol() -> Symbol {
    Symbol::qualified("forge", "contract-card")
}
