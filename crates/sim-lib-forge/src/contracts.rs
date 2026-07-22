use sim_kernel::{
    Cx, Diagnostic, Expr, NumberLiteral, Ref, Result, RuntimeId, Symbol, Value,
    card::card_for_ref,
    library::{ExportKind, ExportRecord, ExportState, LoadedLib},
};
use sim_value::access::entry_field;

/// A compact, source-free model-facing contract for one loaded export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractCard {
    /// Library that contributed the export.
    pub lib: Symbol,
    /// Export kind, carried as the kernel's open export-kind symbol.
    pub export_kind: Symbol,
    /// Export symbol.
    pub symbol: Symbol,
    /// Callable argument Shape encoded as data when known.
    pub args_shape: Option<Expr>,
    /// Callable result Shape encoded as data when known.
    pub result_shape: Option<Expr>,
    /// Library manifest capability requests, as `capability/*` symbols.
    pub capability_symbols: Vec<Symbol>,
    /// Browse Card `requires` data, when present.
    pub card_requires: Option<Expr>,
    /// Human-facing Card summary text.
    pub summary: String,
    /// Example expression, authored or synthesized from the callable Shape.
    pub example: Option<Expr>,
    /// Missing or synthesized pieces retained without dropping the export.
    pub partial: Vec<ContractGap>,
}

/// A missing or synthesized part of a [`ContractCard`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractGap {
    /// The export is callable-shaped but does not expose both argument and result Shapes.
    MissingCallableShape,
    /// The browse Card did not provide summary text.
    MissingCard,
    /// No authored example was available and none could be synthesized.
    MissingExample,
    /// No authored example was available, so FORGE synthesized one from Shape data.
    SynthesizedExample,
}

/// A stable, ordered deck of runtime contract cards and assembly diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractDeck {
    /// Cards assembled from loaded registry exports.
    pub cards: Vec<ContractCard>,
    /// Non-fatal diagnostics recorded while preserving partial exports.
    pub diagnostics: Vec<Diagnostic>,
}

/// Assembles a source-free contract deck from the live registry and browse Cards.
pub fn assemble_contract_deck(cx: &mut Cx) -> Result<ContractDeck> {
    let loaded_libs = cx.registry().libs().to_vec();
    let mut cards = Vec::new();
    let mut diagnostics = Vec::new();

    for loaded in loaded_libs {
        let capability_symbols = loaded
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.as_symbol())
            .collect::<Vec<_>>();

        for export in &loaded.exports {
            let runtime_value = export_value(cx, export);
            let callable = runtime_value
                .as_ref()
                .and_then(|value| value.object().as_callable());
            let mut partial = Vec::new();

            let (args_shape_ref, args_shape, result_shape) = if let Some(callable) = callable {
                let args_shape_ref = callable.browse_args_shape(cx)?;
                let result_shape_ref = callable.browse_result_shape(cx)?;
                let args_shape = args_shape_ref
                    .as_ref()
                    .map(|shape| shape.object().as_expr(cx))
                    .transpose()?;
                let result_shape = result_shape_ref
                    .as_ref()
                    .map(|shape| shape.object().as_expr(cx))
                    .transpose()?;
                if args_shape.is_none() || result_shape.is_none() {
                    partial.push(ContractGap::MissingCallableShape);
                }
                (args_shape_ref, args_shape, result_shape)
            } else {
                if export.kind.name() == Some(ExportKind::FUNCTION) {
                    partial.push(ContractGap::MissingCallableShape);
                }
                (None, None, None)
            };

            let card_fields = browse_card_fields(cx, export.symbol.clone())?;
            let summary = summary_from_card(&card_fields);
            if summary.is_empty() {
                partial.push(ContractGap::MissingCard);
            }
            let card_requires = card_requires_from_card(&card_fields);
            let example = match example_from_card(&card_fields) {
                Some(example) => Some(example),
                None if callable.is_some() => {
                    partial.push(ContractGap::SynthesizedExample);
                    Some(synthesize_example(
                        cx,
                        &export.symbol,
                        args_shape_ref.as_ref(),
                        args_shape.as_ref(),
                    )?)
                }
                None => {
                    partial.push(ContractGap::MissingExample);
                    None
                }
            };

            record_partial_diagnostics(&mut diagnostics, &loaded, export, &partial);
            cards.push(ContractCard {
                lib: loaded.manifest.id.clone(),
                export_kind: export.kind.symbol().clone(),
                symbol: export.symbol.clone(),
                args_shape,
                result_shape,
                capability_symbols: capability_symbols.clone(),
                card_requires,
                summary,
                example,
                partial,
            });
        }
    }

    cards.sort_by(|left, right| {
        (&left.lib, &left.export_kind, &left.symbol).cmp(&(
            &right.lib,
            &right.export_kind,
            &right.symbol,
        ))
    });
    Ok(ContractDeck { cards, diagnostics })
}

pub(crate) fn export_value(cx: &Cx, export: &ExportRecord) -> Option<Value> {
    let id = cx
        .registry()
        .export_symbols()
        .get(&export.kind)?
        .get(&export.symbol)?;
    match id {
        RuntimeId::Class(id) => cx.registry().class_value(*id).cloned(),
        RuntimeId::Function(id) => cx.registry().function_value(*id).cloned(),
        RuntimeId::Macro(id) => cx.registry().macro_value(*id).cloned(),
        RuntimeId::Shape(id) => cx.registry().shape_value(*id).cloned(),
        RuntimeId::Codec(id) => cx.registry().codec_value(*id).cloned(),
        RuntimeId::NumberDomain(id) => cx.registry().number_domain_value(*id).cloned(),
        RuntimeId::Site(_) => cx.registry().site_value(*id).cloned(),
        RuntimeId::Value => cx.registry().value_by_symbol(&export.symbol).cloned(),
    }
}

fn browse_card_fields(cx: &mut Cx, symbol: Symbol) -> Result<Vec<(Expr, Expr)>> {
    let card = card_for_ref(cx, Ref::Symbol(symbol))?;
    match card.object().as_expr(cx)? {
        Expr::Map(entries) => Ok(entries),
        _ => Ok(Vec::new()),
    }
}

fn summary_from_card(entries: &[(Expr, Expr)]) -> String {
    let summary = entry_field(entries, "summary").or_else(|| entry_field(entries, "help"));
    match summary {
        Some(Expr::String(summary)) => summary.trim().to_owned(),
        _ => String::new(),
    }
}

fn card_requires_from_card(entries: &[(Expr, Expr)]) -> Option<Expr> {
    match entry_field(entries, "requires") {
        Some(Expr::List(items)) if !items.is_empty() => Some(Expr::List(items.clone())),
        Some(expr) if !matches!(expr, Expr::Nil) => Some(expr.clone()),
        _ => None,
    }
}

fn example_from_card(entries: &[(Expr, Expr)]) -> Option<Expr> {
    ["example", "expr"]
        .iter()
        .find_map(|field| match entry_field(entries, field) {
            Some(expr) if !matches!(expr, Expr::Nil) => Some(expr.clone()),
            _ => None,
        })
}

fn synthesize_example(
    cx: &mut Cx,
    symbol: &Symbol,
    args_shape_ref: Option<&Value>,
    args_shape: Option<&Expr>,
) -> Result<Expr> {
    Ok(Expr::Call {
        operator: Box::new(Expr::Symbol(symbol.clone())),
        args: synthesize_args(cx, args_shape_ref, args_shape)?,
    })
}

fn synthesize_args(
    cx: &mut Cx,
    args_shape_ref: Option<&Value>,
    args_shape: Option<&Expr>,
) -> Result<Vec<Expr>> {
    let mut candidates = Vec::new();
    if let Some(args_shape) = args_shape {
        candidates.push(args_from_shape_expr(args_shape));
    }
    candidates.extend([
        Vec::new(),
        vec![Expr::String("example".to_owned())],
        vec![Expr::Bool(true)],
        vec![Expr::Number(NumberLiteral {
            domain: Symbol::qualified("core", "Number"),
            canonical: "0".to_owned(),
        })],
        vec![Expr::Symbol(Symbol::new("example"))],
        vec![Expr::Nil],
    ]);

    let Some(args_shape_ref) = args_shape_ref else {
        return Ok(candidates.into_iter().next().unwrap_or_default());
    };
    let Some(shape) = args_shape_ref.object().as_shape() else {
        return Ok(candidates.into_iter().next().unwrap_or_default());
    };
    for args in candidates {
        let expr = Expr::List(args.clone());
        if shape.check_expr(cx, &expr)?.accepted {
            return Ok(args);
        }
    }
    Ok(Vec::new())
}

fn args_from_shape_expr(shape: &Expr) -> Vec<Expr> {
    let Expr::List(items) = shape else {
        return Vec::new();
    };
    let Some(Expr::Symbol(head)) = items.first() else {
        return Vec::new();
    };
    if shape_name(head) != Some("list") {
        return Vec::new();
    }
    let shape_items = if head.namespace.as_deref() == Some("shape") {
        match items.get(1) {
            Some(Expr::List(items)) => items.as_slice(),
            _ => &[],
        }
    } else {
        &items[1..]
    };
    shape_items.iter().map(default_expr_for_shape).collect()
}

fn default_expr_for_shape(shape: &Expr) -> Expr {
    match shape {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => match symbol.name.as_ref() {
            "String" => Expr::String("example".to_owned()),
            "Bool" => Expr::Bool(true),
            "Number" => Expr::Number(NumberLiteral {
                domain: Symbol::qualified("core", "Number"),
                canonical: "0".to_owned(),
            }),
            "Symbol" => Expr::Symbol(Symbol::new("example")),
            "List" => Expr::List(Vec::new()),
            "Map" => Expr::Map(Vec::new()),
            _ => Expr::Nil,
        },
        Expr::Symbol(symbol)
            if symbol.namespace.as_deref() == Some("core") && symbol.name.as_ref() == "Number" =>
        {
            Expr::Number(NumberLiteral {
                domain: Symbol::qualified("core", "Number"),
                canonical: "0".to_owned(),
            })
        }
        _ => Expr::Nil,
    }
}

fn record_partial_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    loaded: &LoadedLib,
    export: &ExportRecord,
    partial: &[ContractGap],
) {
    for gap in partial {
        diagnostics.push(Diagnostic::info(format!(
            "{} {} from {} has {}",
            export.kind.symbol(),
            export.symbol,
            loaded.manifest.id,
            gap.as_symbol()
        )));
    }
    match &export.state {
        ExportState::Resolved { .. } | ExportState::Declared => {}
        ExportState::Unsupported { reason } => diagnostics.push(Diagnostic::info(format!(
            "{} {} is unsupported: {reason}",
            export.kind.symbol(),
            export.symbol
        ))),
        ExportState::Invalid { error } => diagnostics.push(Diagnostic::error(format!(
            "{} {} is invalid: {error}",
            export.kind.symbol(),
            export.symbol
        ))),
    }
}

fn shape_name(symbol: &Symbol) -> Option<&str> {
    if symbol.namespace.is_none() || symbol.namespace.as_deref() == Some("shape") {
        Some(symbol.name.as_ref())
    } else {
        None
    }
}
