use crate::{
    EndpointCard, HarnessCard, PrincipalCard, ProviderAdapter, ProviderFamilyCard,
    ProviderRegistry, ProviderSeatCard, ProviderSeatId, ProviderSeatLimits,
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

struct MultiSeatAdapter {
    family: &'static str,
    labels: &'static [&'static str],
}

impl ProviderAdapter for MultiSeatAdapter {
    fn family(&self) -> ProviderFamilyCard {
        family_card_named(self.family)
    }

    fn discover(&self, _cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        Ok(self
            .labels
            .iter()
            .map(|label| seat_card_named(self.family, label))
            .collect())
    }

    fn open(
        &self,
        _cx: &mut Cx,
        seat: &ProviderSeatCard,
        _options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        Err(Error::Eval(format!("opened {}", seat.seat)))
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

#[test]
fn registry_preserves_and_opens_two_seats_for_one_family_separately() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Arc::new(MultiSeatAdapter {
            family: FICTIONAL_FAMILY,
            labels: &["violet", "amber"],
        }))
        .unwrap();
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let discovered = registry.discover(&mut cx, Expr::Nil).unwrap();

    assert_eq!(registry.families().len(), 1);
    assert_eq!(discovered.len(), 2);
    assert_eq!(registry.seats().len(), 2);
    for label in ["violet", "amber"] {
        let id =
            ProviderSeatId::new(Symbol::qualified("provider", FICTIONAL_FAMILY), label).unwrap();
        assert_eq!(registry.show_seat(&id).unwrap().seat, id);
        let error = registry
            .open(&mut cx, &id, Expr::Nil)
            .err()
            .expect("fixture open reports the selected seat");
        assert!(error.to_string().contains(&format!("opened {id}")));
    }

    let replacement = seat_card_named(FICTIONAL_FAMILY, "violet");
    registry.replace_seat_for_test(replacement.clone());
    assert_eq!(registry.show_seat(&replacement.seat), Some(replacement));
}

#[test]
fn removing_one_adapter_does_not_change_another_family() {
    let mut registry = ProviderRegistry::new();
    for family in ["north-star", "south-star"] {
        registry
            .register(Arc::new(MultiSeatAdapter {
                family,
                labels: &["primary"],
            }))
            .unwrap();
    }
    registry.remove_family(&Symbol::qualified("provider", "north-star"));
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let seats = registry.discover(&mut cx, Expr::Nil).unwrap();

    assert_eq!(registry.families().len(), 1);
    assert_eq!(seats.len(), 1);
    assert_eq!(seats[0].family, Symbol::qualified("provider", "south-star"));
}

#[test]
fn registry_refuses_duplicate_ids_and_has_no_vendor_switch_or_preference() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Arc::new(MultiSeatAdapter {
            family: FICTIONAL_FAMILY,
            labels: &["same", "same"],
        }))
        .unwrap();
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    assert!(registry.discover(&mut cx, Expr::Nil).is_err());

    let source = include_str!("registry.rs");
    assert!(!source.contains("enum Provider"));
    assert!(!source.contains("match family"));
    assert!(!source.contains("openai"));
    assert!(!source.contains("anthropic"));
    assert!(!source.contains(".first()"));
    assert!(!source.contains(".next()"));
}

fn family_card() -> ProviderFamilyCard {
    family_card_named(FICTIONAL_FAMILY)
}

fn family_card_named(family_name: &str) -> ProviderFamilyCard {
    ProviderFamilyCard {
        family: Symbol::qualified("provider", family_name),
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
    seat_card_named(FICTIONAL_FAMILY, DISCOVERED_LABEL)
}

fn seat_card_named(family_name: &str, label: &str) -> ProviderSeatCard {
    let family = Symbol::qualified("provider", family_name);
    ProviderSeatCard {
        seat: ProviderSeatId::new(family.clone(), label).unwrap(),
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
