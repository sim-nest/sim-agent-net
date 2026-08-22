//! Named direct-API provider families and independently authenticated seats.

use crate::{HttpRunner, ProviderAuth, ProviderConfig, ProviderProfile, provider_profiles};
use sim_kernel::{Cx, Error, Expr, Ref, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use sim_lib_provider::{
    CredentialSource, EndpointCard, PrincipalCard, ProviderAdapter, ProviderFamilyCard,
    ProviderRegistry, ProviderSeatCard, ProviderSeatId, ProviderSeatLimits, SecretProviderRegistry,
};
use std::sync::Arc;

/// The explicitly selected OpenAI request wire for one seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenAiWire {
    /// `/chat/completions` request and response shapes.
    Chat,
    /// `/responses` request and response shapes.
    Responses,
}

impl OpenAiWire {
    fn symbol(self) -> Symbol {
        Symbol::new(match self {
            Self::Chat => "openai-chat",
            Self::Responses => "openai-responses",
        })
    }
}

/// Typed authentication contract for a direct API seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectApiAuth {
    /// OpenAI-style bearer API key.
    Bearer(Ref),
    /// Anthropic `x-api-key` credential.
    AnthropicKey(Ref),
}

impl DirectApiAuth {
    fn source(&self) -> CredentialSource {
        CredentialSource::SecretProvider(match self {
            Self::Bearer(r) | Self::AnthropicKey(r) => r.clone(),
        })
    }
}

/// Configuration for one named principal on a direct provider family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectApiSeat {
    /// Stable, redaction-safe principal label.
    pub principal: String,
    /// Model selected by this seat.
    pub model: String,
    /// Opaque credential binding with a family-specific auth shape.
    pub auth: DirectApiAuth,
    /// Required for OpenAI seats and absent for Anthropic seats.
    pub openai_wire: Option<OpenAiWire>,
}

impl DirectApiSeat {
    /// Creates an OpenAI direct-API seat.
    pub fn openai(
        principal: impl Into<String>,
        model: impl Into<String>,
        credential: Ref,
        wire: OpenAiWire,
    ) -> Self {
        Self {
            principal: principal.into(),
            model: model.into(),
            auth: DirectApiAuth::Bearer(credential),
            openai_wire: Some(wire),
        }
    }

    /// Creates an Anthropic direct-API seat.
    pub fn anthropic(
        principal: impl Into<String>,
        model: impl Into<String>,
        credential: Ref,
    ) -> Self {
        Self {
            principal: principal.into(),
            model: model.into(),
            auth: DirectApiAuth::AnthropicKey(credential),
            openai_wire: None,
        }
    }
}

/// Adapter for one named direct API family.
pub struct DirectApiAdapter {
    profile: ProviderProfile,
    seats: Vec<DirectApiSeat>,
    secrets: Arc<SecretProviderRegistry>,
}

impl DirectApiAdapter {
    /// Validates direct seats without resolving credential material.
    pub fn new(
        profile: ProviderProfile,
        seats: Vec<DirectApiSeat>,
        secrets: Arc<SecretProviderRegistry>,
    ) -> Result<Self> {
        match profile.provider.name.as_ref() {
            "openai"
                if seats.iter().all(|s| {
                    matches!(s.auth, DirectApiAuth::Bearer(_)) && s.openai_wire.is_some()
                }) => {}
            "anthropic"
                if seats.iter().all(|s| {
                    matches!(s.auth, DirectApiAuth::AnthropicKey(_)) && s.openai_wire.is_none()
                }) => {}
            "openai" | "anthropic" => {
                return Err(Error::Eval(format!(
                    "provider/{} seat has the wrong authentication or wire shape",
                    profile.provider.name
                )));
            }
            _ => {
                return Err(Error::Eval(format!(
                    "{} is not a named direct-API family",
                    profile.provider
                )));
            }
        }
        if seats.iter().any(|s| s.principal.is_empty()) {
            return Err(Error::Eval(
                "direct-API principal label must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            profile,
            seats,
            secrets,
        })
    }

    fn family_symbol(&self) -> Symbol {
        Symbol::qualified("provider", format!("{}-api", self.profile.provider.name))
    }
}

impl ProviderAdapter for DirectApiAdapter {
    fn family(&self) -> ProviderFamilyCard {
        let wires = if self.profile.provider == Symbol::new("openai") {
            vec![Symbol::new("openai-chat"), Symbol::new("openai-responses")]
        } else {
            vec![Symbol::new("anthropic-messages")]
        };
        ProviderFamilyCard {
            family: self.family_symbol(),
            transport: Symbol::new("http"),
            semantics: Symbol::new("model-turn"),
            auth_owner: Symbol::new("sim"),
            wires,
            operations: vec![
                Symbol::new("discover"),
                Symbol::new("open"),
                Symbol::new("probe"),
            ],
            revision: Expr::Nil,
            extra: vec![(
                Expr::Symbol(Symbol::new("credential-extension")),
                Expr::Symbol(Symbol::new("family-or-seat")),
            )],
        }
    }

    fn discover(&self, _cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        self.seats
            .iter()
            .map(|seat| {
                let family = self.family_symbol();
                let wire = seat
                    .openai_wire
                    .map(OpenAiWire::symbol)
                    .unwrap_or_else(|| Symbol::new("anthropic-messages"));
                Ok(ProviderSeatCard {
                    seat: ProviderSeatId::new(family.clone(), &seat.principal)?,
                    family,
                    principal: PrincipalCard {
                        label: seat.principal.clone(),
                        kind: Symbol::new("api-key"),
                        source: Symbol::new("secret-provider"),
                        digest: "opaque-seat-binding".to_owned(),
                        extra: Vec::new(),
                    },
                    endpoint: EndpointCard {
                        address: self.profile.default_endpoint.clone(),
                        transport: Symbol::new("https"),
                        revision: Expr::Nil,
                        extra: Vec::new(),
                    },
                    harness: None,
                    model: Some(seat.model.clone()),
                    limits: ProviderSeatLimits::default(),
                    revision: Expr::Nil,
                    extra: vec![(Expr::Symbol(Symbol::new("wire")), Expr::Symbol(wire))],
                })
            })
            .collect()
    }

    fn open(
        &self,
        cx: &mut Cx,
        card: &ProviderSeatCard,
        options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        if options != Expr::Nil {
            return Err(Error::Eval(
                "direct-API provider/open accepts nil options".to_owned(),
            ));
        }
        let seat = self
            .seats
            .iter()
            .find(|s| s.principal == card.principal.label)
            .ok_or_else(|| Error::Eval(format!("provider seat {} is not configured", card.seat)))?;
        let secret = self
            .secrets
            .resolve(cx, &seat.auth.source())?
            .ok_or_else(|| Error::Eval("direct-API seat requires authentication".to_owned()))?;
        let mut profile = self.profile.clone();
        if seat.openai_wire == Some(OpenAiWire::Responses) {
            profile.chat_path = "/responses";
            profile.codec = Symbol::qualified("codec", "openai-responses");
        }
        let config = ProviderConfig::for_seat(
            profile.clone(),
            profile.default_endpoint.clone(),
            seat.model.clone(),
            Some(secret),
        )?;
        Ok(Arc::new(HttpRunner::new_provider(config)))
    }
}

/// Registers the named OpenAI and Anthropic direct-API families.
pub fn register_direct_api_families(
    registry: &mut ProviderRegistry,
    openai: Vec<DirectApiSeat>,
    anthropic: Vec<DirectApiSeat>,
    secrets: Arc<SecretProviderRegistry>,
) -> Result<()> {
    registry.register(Arc::new(DirectApiAdapter::new(
        provider_profiles::openai(),
        openai,
        Arc::clone(&secrets),
    )?))?;
    registry.register(Arc::new(DirectApiAdapter::new(
        provider_profiles::anthropic(),
        anthropic,
        secrets,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{DefaultFactory, EagerPolicy};
    use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};
    use sim_lib_provider::{Fanout, FanoutMode, FanoutSeat, PlannedSeat};
    use sim_lib_server::ThreadMode;

    fn cx() -> Cx {
        Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
    }
    fn reference(name: &str) -> Ref {
        Ref::Symbol(Symbol::qualified("secret", name))
    }

    #[test]
    fn named_families_keep_multiple_principals_and_explicit_wires() {
        let mut registry = ProviderRegistry::new();
        register_direct_api_families(
            &mut registry,
            vec![
                DirectApiSeat::openai("oa-one", "gpt-a", reference("oa1"), OpenAiWire::Chat),
                DirectApiSeat::openai("oa-two", "gpt-b", reference("oa2"), OpenAiWire::Responses),
            ],
            vec![
                DirectApiSeat::anthropic("an-one", "claude-a", reference("an1")),
                DirectApiSeat::anthropic("an-two", "claude-b", reference("an2")),
            ],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();
        let families = registry.families();
        assert_eq!(
            families
                .iter()
                .map(|f| f.family.to_string())
                .collect::<Vec<_>>(),
            vec!["provider/anthropic-api", "provider/openai-api"]
        );
        assert!(families.iter().all(
            |f| f.semantics == Symbol::new("model-turn") && f.auth_owner == Symbol::new("sim")
        ));
        let seats = registry.discover(&mut cx(), Expr::Nil).unwrap();
        assert_eq!(seats.len(), 4);
        assert_eq!(
            seats
                .iter()
                .filter(|s| s.family == Symbol::qualified("provider", "openai-api"))
                .map(|s| wire(s))
                .collect::<Vec<_>>(),
            vec!["openai-chat", "openai-responses"]
        );
    }

    #[test]
    fn missing_and_wrong_auth_fail_closed() {
        assert!(
            DirectApiAdapter::new(
                provider_profiles::openai(),
                vec![DirectApiSeat::anthropic("wrong", "model", reference("x"))],
                Arc::new(SecretProviderRegistry::new())
            )
            .is_err()
        );
        let adapter = DirectApiAdapter::new(
            provider_profiles::openai(),
            vec![DirectApiSeat::openai(
                "missing",
                "model",
                reference("missing"),
                OpenAiWire::Chat,
            )],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();
        let card = adapter.discover(&mut cx(), Expr::Nil).unwrap().remove(0);
        assert!(adapter.open(&mut cx(), &card, Expr::Nil).is_err());
    }

    #[test]
    fn mixed_openai_wire_seats_are_distinct_fanout_targets() {
        let adapter = DirectApiAdapter::new(
            provider_profiles::openai(),
            vec![
                DirectApiSeat::openai("chat-key", "gpt", reference("one"), OpenAiWire::Chat),
                DirectApiSeat::openai(
                    "responses-key",
                    "gpt",
                    reference("two"),
                    OpenAiWire::Responses,
                ),
            ],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();
        let seats = adapter.discover(&mut cx(), Expr::Nil).unwrap();
        assert_ne!(seats[0].seat, seats[1].seat);
        assert_eq!(
            [wire(&seats[0]), wire(&seats[1])],
            ["openai-chat", "openai-responses"]
        );
        let targets = seats
            .iter()
            .map(|seat| AnsweringSeat(seat.seat.to_string()))
            .collect::<Vec<_>>();
        let report = Fanout::default().execute(
            &mut cx(),
            &[&targets[0], &targets[1]],
            FanoutMode::All,
            ThreadMode::Pool,
            ModelRequest::default(),
        );
        assert_eq!(report.rows.len(), 2);
        assert!(report.rows.iter().all(|row| row.result.is_ok()));
    }

    struct AnsweringSeat(String);

    impl FanoutSeat for AnsweringSeat {
        fn seat_id(&self) -> &str {
            &self.0
        }

        fn plan(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<PlannedSeat> {
            let id = self.0.clone();
            Ok(PlannedSeat::parallel(
                move || Ok::<_, Error>(id),
                |_cx, id| {
                    Ok(ModelResponse {
                        model: id,
                        ..ModelResponse::default()
                    })
                },
            ))
        }
    }

    fn wire(seat: &ProviderSeatCard) -> &str {
        seat.extra
            .iter()
            .find_map(|(k, v)| match (k, v) {
                (Expr::Symbol(k), Expr::Symbol(v)) if k == &Symbol::new("wire") => {
                    Some(v.name.as_ref())
                }
                _ => None,
            })
            .unwrap()
    }
}
