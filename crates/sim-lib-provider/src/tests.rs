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
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelUsage};
use std::sync::Arc;

struct ConformingRunner {
    identity: &'static str,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl ModelRunner for ConformingRunner {
    fn card(&self) -> sim_lib_agent_runner_core::ModelCard {
        sim_lib_agent_runner_core::ModelCard::default()
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let case = request.extra.iter().find_map(|(key, value)| {
            (key == &Expr::Symbol(Symbol::qualified("provider", "conformance-case")))
                .then_some(value)
        });
        match case {
            Some(Expr::Symbol(case)) if case.name.as_ref() == "error" => {
                Err(Error::Eval("fixture-error".to_owned()))
            }
            Some(Expr::Symbol(case)) if case.name.as_ref() == "cancel" => {
                Err(Error::Eval("cancelled-before-effect".to_owned()))
            }
            _ => {
                let mut response = ModelResponse::new(
                    Symbol::qualified("runner", self.identity),
                    "conformance-model",
                    vec![Expr::String("checked-answer".to_owned())],
                    Symbol::new("stop"),
                );
                response.usage = Some(ModelUsage::default());
                response.extra.extend([
                    (
                        Expr::Symbol(Symbol::qualified("provider", "tool-call")),
                        Expr::Bool(true),
                    ),
                    (
                        Expr::Symbol(Symbol::qualified("provider", "workspace-effect")),
                        Expr::Bool(true),
                    ),
                ]);
                Ok(response)
            }
        }
    }
}

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

include!("tests/registry.rs");
include!("tests/auth.rs");
