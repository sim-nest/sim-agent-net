//! Provider adapters for independently configured local HTTP services.

use crate::{HttpRunner, ProviderAuth, ProviderConfig, ProviderProfile, provider_profiles};
use sim_kernel::{Cx, Error, Expr, Ref, Result, Symbol};
use sim_lib_agent_runner_core::ModelRunner;
use sim_lib_net_core::parse_url;
use sim_lib_provider::{
    CredentialSource, EndpointCard, PrincipalCard, ProviderAdapter, ProviderFamilyCard,
    ProviderRegistry, ProviderSeatCard, ProviderSeatId, ProviderSeatLimits, SecretProviderRegistry,
};
use std::sync::Arc;

/// Configuration for one independently selectable local-service endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalServiceEndpoint {
    /// HTTP base endpoint. Its normalized spelling determines stable seat identity.
    pub endpoint: String,
    /// Model selected when this seat is opened.
    pub model: String,
    /// Optional preopened credential source. Only LM Studio accepts one.
    pub credential: CredentialSource,
}

impl LocalServiceEndpoint {
    /// Creates a no-auth endpoint using the profile's default model.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            credential: CredentialSource::None,
        }
    }

    /// Binds an opaque, preopened secret-provider reference to this seat.
    pub fn with_bearer_credential(mut self, reference: Ref) -> Self {
        self.credential = CredentialSource::SecretProvider(reference);
        self
    }
}

/// HTTP adapter for one local-service family and all of its configured endpoints.
pub struct LocalServiceAdapter {
    profile: ProviderProfile,
    endpoints: Vec<LocalServiceEndpoint>,
    secrets: Arc<SecretProviderRegistry>,
}

impl LocalServiceAdapter {
    /// Admits endpoint configuration without probing or changing daemon state.
    pub fn new(
        profile: ProviderProfile,
        endpoints: Vec<LocalServiceEndpoint>,
        secrets: Arc<SecretProviderRegistry>,
    ) -> Result<Self> {
        if !matches!(
            profile.provider.name.as_ref(),
            "ollama" | "lemonade" | "lm-studio"
        ) {
            return Err(Error::Eval(format!(
                "{} is not a local-service provider family",
                profile.provider
            )));
        }
        for endpoint in &endpoints {
            parse_url(&endpoint.endpoint)
                .map_err(|error| Error::Eval(format!("invalid provider endpoint: {error}")))?;
            if !matches!(profile.auth, ProviderAuth::OptionalBearerEnv { .. })
                && endpoint.credential != CredentialSource::None
            {
                return Err(Error::Eval(format!(
                    "provider/{} local-service seats do not accept credentials",
                    profile.provider.name
                )));
            }
        }
        Ok(Self {
            profile,
            endpoints,
            secrets,
        })
    }

    fn family_symbol(&self) -> Symbol {
        Symbol::qualified("provider", self.profile.provider.name.as_ref())
    }
}

impl ProviderAdapter for LocalServiceAdapter {
    fn family(&self) -> ProviderFamilyCard {
        ProviderFamilyCard {
            family: self.family_symbol(),
            transport: Symbol::new("http"),
            semantics: Symbol::new("model-turn"),
            auth_owner: if matches!(self.profile.auth, ProviderAuth::OptionalBearerEnv { .. }) {
                Symbol::new("seat")
            } else {
                Symbol::new("none")
            },
            wires: vec![self.profile.codec.clone()],
            operations: vec![
                Symbol::new("discover"),
                Symbol::new("open"),
                Symbol::new("probe"),
            ],
            revision: Expr::Nil,
            extra: vec![(
                Expr::Symbol(Symbol::new("lifecycle")),
                Expr::Symbol(Symbol::new("external")),
            )],
        }
    }

    fn discover(&self, _cx: &mut Cx, _hint: Expr) -> Result<Vec<ProviderSeatCard>> {
        self.endpoints
            .iter()
            .map(|endpoint| seat_card(&self.profile, endpoint))
            .collect()
    }

    fn open(
        &self,
        cx: &mut Cx,
        seat: &ProviderSeatCard,
        options: Expr,
    ) -> Result<Arc<dyn ModelRunner>> {
        if options != Expr::Nil {
            return Err(Error::Eval(
                "local-service provider/open accepts nil options".to_owned(),
            ));
        }
        let configured = self
            .endpoints
            .iter()
            .find(|endpoint| {
                seat_id(&self.profile, &endpoint.endpoint)
                    .is_ok_and(|configured| configured == seat.seat)
            })
            .ok_or_else(|| Error::Eval(format!("provider seat {} is not configured", seat.seat)))?;
        let secret = self.secrets.resolve(cx, &configured.credential)?;
        let config = ProviderConfig::for_seat(
            self.profile.clone(),
            configured.endpoint.clone(),
            configured.model.clone(),
            secret,
        )?;
        Ok(Arc::new(HttpRunner::new_provider(config)))
    }
}

/// Registers Ollama, Lemonade, and LM Studio local-service families.
pub fn register_local_service_families(
    registry: &mut ProviderRegistry,
    ollama: Vec<LocalServiceEndpoint>,
    lemonade: Vec<LocalServiceEndpoint>,
    lm_studio: Vec<LocalServiceEndpoint>,
    secrets: Arc<SecretProviderRegistry>,
) -> Result<()> {
    for (profile, endpoints) in [
        (provider_profiles::ollama(), ollama),
        (provider_profiles::lemonade(), lemonade),
        (provider_profiles::lm_studio(), lm_studio),
    ] {
        registry.register(Arc::new(LocalServiceAdapter::new(
            profile,
            endpoints,
            Arc::clone(&secrets),
        )?))?;
    }
    Ok(())
}

fn seat_card(
    profile: &ProviderProfile,
    configured: &LocalServiceEndpoint,
) -> Result<ProviderSeatCard> {
    let parts = parse_url(&configured.endpoint)
        .map_err(|error| Error::Eval(format!("invalid provider endpoint: {error}")))?;
    let posture = if is_loopback(&parts.host) {
        "loopback"
    } else {
        "network"
    };
    let credential_bound = matches!(configured.credential, CredentialSource::SecretProvider(_));
    let family = Symbol::qualified("provider", profile.provider.name.as_ref());
    Ok(ProviderSeatCard {
        seat: seat_id(profile, &configured.endpoint)?,
        family,
        principal: PrincipalCard {
            label: if credential_bound {
                "seat-bound"
            } else {
                "none"
            }
            .to_owned(),
            kind: if credential_bound {
                Symbol::new("bearer")
            } else {
                Symbol::new("none")
            },
            source: if credential_bound {
                Symbol::new("secret-provider")
            } else {
                Symbol::new("none")
            },
            digest: if credential_bound {
                "opaque-seat-binding"
            } else {
                "none"
            }
            .to_owned(),
            extra: Vec::new(),
        },
        endpoint: EndpointCard {
            address: configured.endpoint.clone(),
            transport: Symbol::new("http"),
            revision: Expr::Nil,
            extra: vec![
                (
                    Expr::Symbol(Symbol::new("identity")),
                    Expr::String(endpoint_identity(&configured.endpoint)),
                ),
                (
                    Expr::Symbol(Symbol::new("posture")),
                    Expr::Symbol(Symbol::new(posture)),
                ),
            ],
        },
        harness: None,
        model: Some(configured.model.clone()),
        limits: ProviderSeatLimits::default(),
        revision: Expr::Nil,
        extra: vec![(
            Expr::Symbol(Symbol::new("lifecycle")),
            Expr::Symbol(Symbol::new("external")),
        )],
    })
}

fn seat_id(profile: &ProviderProfile, endpoint: &str) -> Result<ProviderSeatId> {
    ProviderSeatId::new(
        Symbol::qualified("provider", profile.provider.name.as_ref()),
        format!("endpoint-{}", endpoint_identity(endpoint)),
    )
}

fn endpoint_identity(endpoint: &str) -> String {
    // FNV-1a is deliberately specified here rather than using process-randomized hashing.
    let hash = endpoint
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

fn is_loopback(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{DefaultFactory, EagerPolicy};
    use sim_lib_agent_runner_core::{ModelRequest, ModelResponse};
    use sim_lib_provider::{Fanout, FanoutMode, FanoutSeat, PlannedSeat};
    use sim_lib_server::ThreadMode;
    use std::time::{Duration, Instant};

    fn cx() -> Cx {
        Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
    }

    #[test]
    fn local_service_families_register_with_model_turn_semantics() {
        let mut registry = ProviderRegistry::new();
        register_local_service_families(
            &mut registry,
            vec![LocalServiceEndpoint::new("http://127.0.0.1:11434", "qwen")],
            vec![LocalServiceEndpoint::new(
                "http://127.0.0.1:13305/v1",
                "coder",
            )],
            vec![LocalServiceEndpoint::new(
                "http://127.0.0.1:1234/v1",
                "local",
            )],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();

        let families = registry.families();
        assert_eq!(families.len(), 3);
        assert!(families.iter().all(|family| {
            family.transport == Symbol::new("http") && family.semantics == Symbol::new("model-turn")
        }));
    }

    #[test]
    fn every_endpoint_is_a_stable_separate_seat_with_honest_posture() {
        let adapter = LocalServiceAdapter::new(
            provider_profiles::ollama(),
            vec![
                LocalServiceEndpoint::new("http://127.0.0.1:11434", "one"),
                LocalServiceEndpoint::new("http://model-box.local:11434", "two"),
            ],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();

        let first = adapter.discover(&mut cx(), Expr::Nil).unwrap();
        let second = adapter.discover(&mut cx(), Expr::Nil).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_ne!(first[0].seat, first[1].seat);
        assert_eq!(
            extra_symbol(&first[0].endpoint.extra, "posture"),
            "loopback"
        );
        assert_eq!(extra_symbol(&first[1].endpoint.extra, "posture"), "network");
        assert!(
            first
                .iter()
                .all(|seat| { extra_symbol(&seat.extra, "lifecycle") == "external" })
        );
    }

    #[test]
    fn discovery_is_inert_for_unavailable_endpoints_and_auth_is_family_specific() {
        let unavailable = LocalServiceEndpoint::new("http://127.0.0.1:1", "missing-model");
        let adapter = LocalServiceAdapter::new(
            provider_profiles::lemonade(),
            vec![unavailable.clone()],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();
        assert_eq!(adapter.discover(&mut cx(), Expr::Nil).unwrap().len(), 1);

        let credential = Ref::Symbol(Symbol::qualified("secret", "lm-studio-test"));
        assert!(
            LocalServiceAdapter::new(
                provider_profiles::ollama(),
                vec![
                    unavailable
                        .clone()
                        .with_bearer_credential(credential.clone())
                ],
                Arc::new(SecretProviderRegistry::new()),
            )
            .is_err()
        );
        assert!(
            LocalServiceAdapter::new(
                provider_profiles::lemonade(),
                vec![unavailable.with_bearer_credential(credential.clone())],
                Arc::new(SecretProviderRegistry::new()),
            )
            .is_err()
        );
        assert!(
            LocalServiceAdapter::new(
                provider_profiles::lm_studio(),
                vec![
                    LocalServiceEndpoint::new("http://127.0.0.1:1234/v1", "local")
                        .with_bearer_credential(credential)
                ],
                Arc::new(SecretProviderRegistry::new()),
            )
            .is_ok()
        );
    }

    #[test]
    fn two_endpoints_of_one_family_answer_in_one_parallel_fanout_report() {
        let adapter = LocalServiceAdapter::new(
            provider_profiles::ollama(),
            vec![
                LocalServiceEndpoint::new("http://127.0.0.1:11434", "one"),
                LocalServiceEndpoint::new("http://127.0.0.1:11435", "two"),
            ],
            Arc::new(SecretProviderRegistry::new()),
        )
        .unwrap();
        let discovered = adapter.discover(&mut cx(), Expr::Nil).unwrap();
        let first = AnsweringSeat::new(discovered[0].seat.to_string(), 90);
        let second = AnsweringSeat::new(discovered[1].seat.to_string(), 90);
        let started = Instant::now();
        let report = Fanout::default().execute(
            &mut cx(),
            &[&first, &second],
            FanoutMode::All,
            ThreadMode::Pool,
            ModelRequest::default(),
        );

        assert!(started.elapsed() < Duration::from_millis(170));
        assert_eq!(report.rows.len(), 2);
        assert_ne!(report.rows[0].seat, report.rows[1].seat);
        assert!(report.rows.iter().all(|row| row.result.is_ok()));
    }

    struct AnsweringSeat {
        id: String,
        delay_ms: u64,
    }

    impl AnsweringSeat {
        fn new(id: String, delay_ms: u64) -> Self {
            Self { id, delay_ms }
        }
    }

    impl FanoutSeat for AnsweringSeat {
        fn seat_id(&self) -> &str {
            &self.id
        }

        fn plan(&self, _cx: &mut Cx, _request: ModelRequest) -> Result<PlannedSeat> {
            let delay_ms = self.delay_ms;
            let id = self.id.clone();
            Ok(PlannedSeat::parallel(
                move || {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    Ok::<_, Error>(id)
                },
                |_cx, id| {
                    Ok(ModelResponse::new(
                        Symbol::new(id),
                        "fixture",
                        vec![Expr::String("answered".to_owned())],
                        Symbol::new("stop"),
                    ))
                },
            ))
        }
    }

    fn extra_symbol(extra: &[(Expr, Expr)], key: &str) -> String {
        extra
            .iter()
            .find_map(|(name, value)| match (name, value) {
                (Expr::Symbol(name), Expr::Symbol(value)) if name == &Symbol::new(key) => {
                    Some(value.to_string())
                }
                _ => None,
            })
            .unwrap()
    }
}
