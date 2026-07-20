use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Claim, ClassRef, Cx, Datum, DefaultFactory,
    EagerPolicy, Export, ExportKind, Expr, ExprKind, Lib, LibManifest, LibTarget, Linker, LoadCx,
    Object, ObjectCompat, Ref, Result, Symbol, Value, Version, card::card_help_predicate,
};
use sim_shape::{ExprKindShape, ListShape, check_shape_on_expr, parse_shape_expr, shape_value};

use crate::{ContractGap, assemble_contract_deck, contract_card_from_expr, contract_card_shape};

#[test]
fn contract_deck_assembles_runtime_cards_and_round_trips() {
    let (mut cx, seat) = Cx::new_seated(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    seat.grant(&mut cx, CapabilityName::new("forge.fixture"))
        .unwrap();
    cx.load_lib(&FixtureContracts).unwrap();
    insert_help_claim(&mut cx, known_symbol(), "shape-known callable").unwrap();

    let deck = assemble_contract_deck(&mut cx).unwrap();
    let known = deck
        .cards
        .iter()
        .find(|card| card.symbol == known_symbol())
        .expect("known export card")
        .clone();
    let partial = deck
        .cards
        .iter()
        .find(|card| card.symbol == partial_symbol())
        .expect("partial export card");

    assert_eq!(known.lib, Symbol::qualified("fixture", "contracts"));
    assert_eq!(known.export_kind, Symbol::new(ExportKind::FUNCTION));
    assert_eq!(
        known.capability_symbols,
        vec![CapabilityName::new("forge.fixture").as_symbol()]
    );
    assert!(known.args_shape.is_some());
    assert!(known.result_shape.is_some());
    assert_eq!(known.summary, "shape-known callable");
    assert!(known.example.is_some());
    let Some(Expr::Call { args, .. }) = known.example.as_ref() else {
        panic!("expected synthesized call example");
    };
    assert_eq!(args, &vec![Expr::String("example".to_owned())]);
    assert_eq!(known.partial, vec![ContractGap::SynthesizedExample]);

    let expr = known.as_expr();
    let shape = parse_shape_expr(&contract_card_shape()).unwrap();
    let checked = check_shape_on_expr(shape.as_ref(), &mut cx, &expr).unwrap();
    assert!(checked.accepted, "{:?}", checked.diagnostics);
    let decoded = contract_card_from_expr(&mut cx, &expr).unwrap();
    assert_eq!(decoded, known);

    assert_eq!(partial.lib, Symbol::qualified("fixture", "contracts"));
    assert!(partial.partial.contains(&ContractGap::MissingCallableShape));
    assert!(partial.partial.contains(&ContractGap::MissingCard));
    assert!(partial.partial.contains(&ContractGap::MissingExample));
}

struct FixtureContracts;

impl Lib for FixtureContracts {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("fixture", "contracts"),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: vec![CapabilityName::new("forge.fixture")],
            exports: vec![
                Export::Function {
                    symbol: known_symbol(),
                    function_id: None,
                },
                Export::Function {
                    symbol: partial_symbol(),
                    function_id: None,
                },
            ],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker) -> Result<()> {
        linker.function_value(
            known_symbol(),
            cx.factory().opaque(Arc::new(KnownCallable))?,
        )?;
        linker.unsupported_export(
            ExportKind::named(ExportKind::FUNCTION),
            partial_symbol(),
            "fixture partial",
        )?;
        Ok(())
    }
}

struct KnownCallable;

impl Object for KnownCallable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<fixture-known-callable>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for KnownCallable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for KnownCallable {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        cx.factory().nil()
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<Value>> {
        Ok(Some(shape_value(
            Symbol::qualified("fixture", "known-args"),
            Arc::new(ListShape::new(vec![Arc::new(ExprKindShape::new(
                ExprKind::String,
            ))])),
        )))
    }

    fn browse_result_shape(&self, _cx: &mut Cx) -> Result<Option<Value>> {
        Ok(Some(shape_value(
            Symbol::qualified("fixture", "known-result"),
            Arc::new(ExprKindShape::new(ExprKind::Bool)),
        )))
    }
}

fn insert_help_claim(cx: &mut Cx, subject: Symbol, help: &str) -> Result<()> {
    let claim = Claim::content_object(
        cx.datum_store_mut(),
        Ref::Symbol(subject),
        card_help_predicate(),
        Datum::String(help.to_owned()),
    )?;
    cx.insert_fact(claim)?;
    Ok(())
}

fn known_symbol() -> Symbol {
    Symbol::qualified("fixture", "known")
}

fn partial_symbol() -> Symbol {
    Symbol::qualified("fixture", "partial")
}
