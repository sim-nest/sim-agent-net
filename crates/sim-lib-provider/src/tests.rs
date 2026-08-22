use crate::{
    EndpointCard, HarnessCard, PrincipalCard, ProviderAdapter, ProviderFamilyCard,
    ProviderSeatCard, ProviderSeatId, ProviderSeatLimits,
};
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use std::sync::Arc;

const FICTIONAL_FAMILY: &str = "nebula-relay";
const DISCOVERED_LABEL: &str = "violet-seat-47";

struct FictionalAdapter;

impl ProviderAdapter for FictionalAdapter {
    fn family(&self) -> ProviderFamilyCard {
        family_card()
    }

    fn discover(&self, _cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        Ok(vec![seat_card()])
    }

    fn open(
        &self,
        _cx: &mut Cx,
        _seat: &ProviderSeatCard,
        _options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        Err(Error::Eval(
            "fictional adapter has no execution backend".to_owned(),
        ))
    }
}

#[test]
fn provider_families_and_discovered_seats_are_open() {
    let adapter = FictionalAdapter;
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));

    assert_eq!(adapter.family().family.name.as_ref(), FICTIONAL_FAMILY);
    let seats = adapter.discover(&mut cx, Expr::Nil).unwrap();
    assert_eq!(seats.len(), 1);
    assert_eq!(
        seats[0].seat.to_string(),
        "seat:nebula-relay#violet-seat-47"
    );
}

#[test]
fn seat_id_rejects_ambiguous_or_unprintable_labels() {
    let family = Symbol::qualified("provider", FICTIONAL_FAMILY);
    for label in ["", "two words", "two#labels", "line\nbreak", "tab\tlabel"] {
        assert!(
            ProviderSeatId::new(family.clone(), label).is_err(),
            "{label:?}"
        );
    }
}

#[test]
fn provider_crate_keeps_model_execution_in_runner_core() {
    let manifest = include_str!("../Cargo.toml");
    let adapter = include_str!("adapter.rs");
    assert!(manifest.contains("sim-lib-agent-runner-core"));
    assert!(!manifest.contains("sim-lib-agent ="));
    assert!(!adapter.contains("fn infer("));
    assert!(!adapter.contains("ModelRequest"));
    assert!(!adapter.contains("ModelResponse"));
}

fn family_card() -> ProviderFamilyCard {
    ProviderFamilyCard {
        family: Symbol::qualified("provider", FICTIONAL_FAMILY),
        transport: Symbol::new("quantum-mailbox"),
        semantics: Symbol::new("model-turn"),
        auth_owner: Symbol::new("sim"),
        wires: vec![Symbol::new("nebula-packets")],
        operations: vec![Symbol::new("probe")],
        revision: Expr::Nil,
        extra: vec![(Expr::Symbol(Symbol::new("fictional")), Expr::Bool(true))],
    }
}

fn seat_card() -> ProviderSeatCard {
    let family = Symbol::qualified("provider", FICTIONAL_FAMILY);
    ProviderSeatCard {
        seat: ProviderSeatId::new(family.clone(), DISCOVERED_LABEL).unwrap(),
        family,
        principal: PrincipalCard {
            label: "fixture".to_owned(),
            kind: Symbol::new("none"),
            source: Symbol::new("none"),
            digest: "redacted-fixture".to_owned(),
            extra: Vec::new(),
        },
        endpoint: EndpointCard {
            address: "nebula://fixture".to_owned(),
            transport: Symbol::new("quantum-mailbox"),
            revision: Expr::Nil,
            extra: Vec::new(),
        },
        harness: Some(HarnessCard {
            kind: Symbol::new("fictional"),
            label: "fixture-harness".to_owned(),
            revision: Expr::Nil,
            extra: Vec::new(),
        }),
        model: Some("constellation-1".to_owned()),
        limits: ProviderSeatLimits::default(),
        revision: Expr::Nil,
        extra: Vec::new(),
    }
}
