use crate::{
    AuthMetadata, AuthMethod, AuthOwner, CredentialSource, EndpointCard, HarnessCard,
    PrincipalCard, ProviderAdapter, ProviderFamilyCard, ProviderRegistry, ProviderSeatCard,
    ProviderSeatId, ProviderSeatLimits, Secret, SecretProvider, SecretProviderRegistry,
    SessionStatus, TermsAcknowledgement,
};
use sim_kernel::{
    CapabilityName, CapabilitySet, Cx, DefaultFactory, EagerPolicy, Error, Expr, Ref, Result,
    Symbol,
};
use sim_lib_agent_runner_core::ModelRunner;
use std::sync::Arc;

enum FakeSecretResult {
    Material(&'static str),
    Error(&'static str),
}

struct FakeSecretProvider(FakeSecretResult);

impl SecretProvider for FakeSecretProvider {
    fn resolve(&self, _cx: &mut Cx) -> Result<Secret> {
        match self.0 {
            FakeSecretResult::Material(material) => Secret::new(material),
            FakeSecretResult::Error(reason) => Err(Error::Eval(reason.to_owned())),
        }
    }
}

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
    let mut cx = test_cx_with_secret_capability();

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
    let mut cx = test_cx_with_secret_capability();
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
    let mut cx = test_cx_with_secret_capability();
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
    let mut cx = test_cx_with_secret_capability();
    assert!(registry.discover(&mut cx, Expr::Nil).is_err());

    let source = include_str!("registry.rs");
    assert!(!source.contains("enum Provider"));
    assert!(!source.contains("match family"));
    assert!(!source.contains("openai"));
    assert!(!source.contains("anthropic"));
    assert!(!source.contains(".first()"));
    assert!(!source.contains(".next()"));
}

#[test]
fn two_preopened_providers_resolve_distinct_seat_credentials_in_one_process() {
    let alpha = Ref::Symbol(Symbol::qualified("secret-provider", "alpha"));
    let beta = Ref::Symbol(Symbol::qualified("secret-provider", "beta"));
    let mut providers = SecretProviderRegistry::new();
    providers
        .bind(
            alpha.clone(),
            Arc::new(FakeSecretProvider(FakeSecretResult::Material("alpha-key"))),
        )
        .unwrap();
    providers
        .bind(
            beta.clone(),
            Arc::new(FakeSecretProvider(FakeSecretResult::Material("beta-key"))),
        )
        .unwrap();
    let mut cx = test_cx_with_secret_capability();

    let capabilities = CapabilitySet::new().grant(CapabilityName::new("ai-runner-secret"));
    let (first, second) = cx
        .with_capabilities(capabilities.clone(), |cx| {
            Ok((
                providers
                    .resolve(cx, &CredentialSource::SecretProvider(alpha))?
                    .unwrap(),
                providers
                    .resolve(cx, &CredentialSource::SecretProvider(beta))?
                    .unwrap(),
            ))
        })
        .unwrap();

    assert_eq!(first.expose(), "alpha-key");
    assert_eq!(second.expose(), "beta-key");
    assert_eq!(
        providers
            .resolve(&mut cx, &CredentialSource::BrokerOwned)
            .unwrap(),
        None
    );
    assert_eq!(
        providers.resolve(&mut cx, &CredentialSource::None).unwrap(),
        None
    );
}

#[test]
fn secret_provider_failures_and_revocation_never_echo_material() {
    let reference = Ref::Symbol(Symbol::qualified("secret-provider", "fixture"));
    let material = "never-print-this-secret";
    let cases = ["refused", "revoked", "timed out"];
    for reason in cases {
        let mut providers = SecretProviderRegistry::new();
        providers
            .bind(
                reference.clone(),
                Arc::new(FakeSecretProvider(FakeSecretResult::Error(reason))),
            )
            .unwrap();
        let mut cx = test_cx_with_secret_capability();
        let capabilities = CapabilitySet::new().grant(CapabilityName::new("ai-runner-secret"));
        let error = cx
            .with_capabilities(capabilities, |cx| {
                providers.resolve(cx, &CredentialSource::SecretProvider(reference.clone()))
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains(reason));
        assert!(!error.contains(material));
    }

    let mut providers = SecretProviderRegistry::new();
    let mut cx = test_cx_with_secret_capability();
    let source = CredentialSource::SecretProvider(reference.clone());
    let capabilities = CapabilitySet::new().grant(CapabilityName::new("ai-runner-secret"));
    let error = cx
        .with_capabilities(capabilities, |cx| providers.resolve(cx, &source))
        .unwrap_err()
        .to_string();
    assert!(!error.contains(material));
    providers
        .bind(
            reference.clone(),
            Arc::new(FakeSecretProvider(FakeSecretResult::Material(material))),
        )
        .unwrap();
    assert!(providers.revoke(&reference));
    let capabilities = CapabilitySet::new().grant(CapabilityName::new("ai-runner-secret"));
    let error = cx
        .with_capabilities(capabilities, |cx| {
            providers.resolve(cx, &CredentialSource::SecretProvider(reference))
        })
        .unwrap_err()
        .to_string();
    assert!(!error.contains(material));
}

#[test]
fn secret_provider_resolution_requires_open_time_authority() {
    let reference = Ref::Symbol(Symbol::qualified("secret-provider", "denied"));
    let mut providers = SecretProviderRegistry::new();
    providers
        .bind(
            reference.clone(),
            Arc::new(FakeSecretProvider(FakeSecretResult::Material("denied-key"))),
        )
        .unwrap();
    let mut cx = test_cx_with_secret_capability();
    let error = providers
        .resolve(&mut cx, &CredentialSource::SecretProvider(reference))
        .unwrap_err()
        .to_string();
    assert!(error.contains("ai-runner-secret"));
    assert!(!error.contains("denied-key"));
}

fn test_cx_with_secret_capability() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x5052_4f56),
    )
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

#[test]
fn auth_vocabulary_and_control_operations_are_stable_and_separate_from_inference() {
    let methods = [
        AuthMethod::ApiKey,
        AuthMethod::OauthBrowser,
        AuthMethod::OauthDevice,
        AuthMethod::Subscription,
        AuthMethod::BrokerOwned,
        AuthMethod::None,
    ];
    assert_eq!(
        methods.map(|method| method.symbol().to_string()),
        [
            "api-key",
            "oauth-browser",
            "oauth-device",
            "subscription",
            "broker-owned",
            "none"
        ]
    );
    assert_eq!(
        crate::provider_operation::all()
            .into_iter()
            .map(|op| op.to_string())
            .collect::<Vec<_>>(),
        [
            "provider/auth-methods",
            "provider/login",
            "provider/status",
            "provider/logout"
        ]
    );
}

#[test]
fn redacted_auth_metadata_round_trips_and_terms_gate_open_and_login() {
    let mut seat = seat_card();
    let metadata = AuthMetadata {
        owner: AuthOwner::Broker,
        session: SessionStatus::LoginRequired,
        required_terms: Some(("provider-terms".into(), "2026-08".into())),
        acknowledgement: None,
    };
    seat.set_auth_metadata(&metadata);
    assert_eq!(seat.auth_metadata().unwrap(), Some(metadata.clone()));
    let encoded = format!("{:?}", seat.extra);
    assert!(!encoded.contains("credential"));
    assert!(!encoded.contains("cookie"));

    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FictionalAdapter)).unwrap();
    registry
        .discover(&mut test_cx_with_secret_capability(), Expr::Nil)
        .unwrap();
    registry.replace_seat_for_test(seat.clone());
    let mut cx = test_cx_with_secret_capability();
    let open_error = match registry.open(&mut cx, &seat.seat, Expr::Nil) {
        Ok(_) => panic!("unacknowledged terms must refuse open"),
        Err(error) => error,
    };
    assert!(open_error.to_string().contains("must be acknowledged"));
    assert!(
        registry
            .login(&mut cx, &seat.seat, AuthMethod::BrokerOwned)
            .unwrap_err()
            .to_string()
            .contains("must be acknowledged")
    );

    let mut accepted = metadata;
    accepted.acknowledgement = Some(TermsAcknowledgement {
        terms_id: "provider-terms".into(),
        revision: "2026-08".into(),
        acknowledged_by: "operator".into(),
    });
    seat.set_auth_metadata(&accepted);
    registry.replace_seat_for_test(seat.clone());
    let adapter_error = match registry.open(&mut cx, &seat.seat, Expr::Nil) {
        Ok(_) => panic!("fictional adapter unexpectedly opened"),
        Err(error) => error,
    };
    assert!(adapter_error.to_string().contains("no execution backend"));
}
