use sim_kernel::{Datum, Error, Result};
use sim_lib_provider::{ProviderFamilyCard, ProviderSeatCard, ProviderSeatId};

/// Artifact evidence strong enough to identify the bytes or hosted revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactEvidence {
    /// Immutable served bytes, including an Ollama manifest digest.
    Digest { algorithm: String, value: String },
    /// Provider/operator assertion for a hosted model revision.
    Epoch(String),
}

/// What product a provider route actually grades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteSemantics {
    LocalService,
    DirectApi,
    BrokeredModel,
    BrokeredAgent,
}

/// Identity-relevant route facts. Endpoint aliases are deliberately excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRoute {
    pub semantics: RouteSemantics,
    pub wire: String,
    pub provider_revision: String,
}

/// Model-visible capacity limits, distinct from provider rate limits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelLimits {
    pub context_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// How strongly the provider can attest the returned product.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IdentityConfidence {
    Product,
    Provider,
    Artifact,
}

/// Complete identity of one callable model product at one provider seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateRevision {
    pub seat: ProviderSeatId,
    pub model: String,
    pub artifact: ArtifactEvidence,
    pub backend: String,
    pub quantization: Option<String>,
    pub model_defaults: Vec<(String, String)>,
    pub modalities: Vec<String>,
    pub limits: ModelLimits,
    pub service_tier: Option<String>,
    pub route: ModelRoute,
    pub external_harness_revision: Option<String>,
    pub confidence: IdentityConfidence,
}

impl CandidateRevision {
    /// Builds identity from a delivered provider seat plus model-specific facts.
    pub fn from_provider(
        family: &ProviderFamilyCard,
        seat: &ProviderSeatCard,
        artifact: ArtifactEvidence,
        backend: impl Into<String>,
        confidence: IdentityConfidence,
    ) -> Result<Self> {
        if family.family != seat.family || family.family != seat.seat.family {
            return Err(Error::Eval(
                "provider family and seat identity disagree".into(),
            ));
        }
        let model = seat.model.clone().ok_or_else(|| {
            Error::Eval("candidate identity requires the provider's selected model".into())
        })?;
        validate_artifact(&artifact)?;
        let semantics = route_semantics(family)?;
        Ok(Self {
            seat: seat.seat.clone(),
            model,
            artifact,
            backend: required(backend.into(), "backend")?,
            quantization: None,
            model_defaults: Vec::new(),
            modalities: Vec::new(),
            limits: ModelLimits::default(),
            service_tier: None,
            route: ModelRoute {
                semantics,
                wire: family
                    .wires
                    .first()
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
                provider_revision: expr_revision(&family.revision)?,
            },
            external_harness_revision: seat
                .harness
                .as_ref()
                .map(|harness| {
                    expr_revision(&harness.revision)
                        .map(|revision| format!("{}:{revision}", harness.kind.name))
                })
                .transpose()?,
            // A brokered agent may transform, route, or synthesize the turn.
            // Without raw-model proof the honest subject is the whole product.
            confidence: if semantics == RouteSemantics::BrokeredAgent {
                IdentityConfidence::Product
            } else {
                confidence
            },
        })
    }

    /// Canonical public datum used as the sole candidate hash input.
    pub fn to_datum(&self) -> Datum {
        Datum::Node {
            tag: sim_kernel::Symbol::qualified("model-test", "candidate-revision-v1"),
            fields: vec![
                field("seat", self.seat.to_string()),
                field("model", self.model.clone()),
                field("artifact", artifact_text(&self.artifact)),
                field("backend", self.backend.clone()),
                field(
                    "quantization",
                    self.quantization.clone().unwrap_or_default(),
                ),
                sorted_list_field(
                    "defaults",
                    &self
                        .model_defaults
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>(),
                ),
                sorted_list_field("modalities", &self.modalities),
                field("context-tokens", option_number(self.limits.context_tokens)),
                field("output-tokens", option_number(self.limits.output_tokens)),
                field(
                    "service-tier",
                    self.service_tier.clone().unwrap_or_default(),
                ),
                field("route", format!("{:?}", self.route.semantics)),
                field("wire", self.route.wire.clone()),
                field("provider-revision", self.route.provider_revision.clone()),
                field(
                    "external-harness",
                    self.external_harness_revision.clone().unwrap_or_default(),
                ),
                field("confidence", format!("{:?}", self.confidence)),
            ],
        }
    }
}

fn route_semantics(family: &ProviderFamilyCard) -> Result<RouteSemantics> {
    let transport = family.transport.name.as_str();
    let semantics = family.semantics.name.as_str();
    match (transport, semantics) {
        ("local-service", "model-turn") | ("http-local", "model-turn") => {
            Ok(RouteSemantics::LocalService)
        }
        ("http", "model-turn") | ("https", "model-turn") => Ok(RouteSemantics::DirectApi),
        (_, "model-turn") => Ok(RouteSemantics::BrokeredModel),
        (_, "agent-task") => Ok(RouteSemantics::BrokeredAgent),
        _ => Err(Error::Eval(format!(
            "unsupported provider route {transport}/{semantics}"
        ))),
    }
}

fn validate_artifact(artifact: &ArtifactEvidence) -> Result<()> {
    match artifact {
        ArtifactEvidence::Digest { algorithm, value }
            if !algorithm.trim().is_empty() && !value.trim().is_empty() =>
        {
            Ok(())
        }
        ArtifactEvidence::Epoch(value) if !value.trim().is_empty() => Ok(()),
        ArtifactEvidence::Digest { .. } => Err(Error::Eval(
            "artifact digest requires algorithm and value".into(),
        )),
        ArtifactEvidence::Epoch(_) => Err(Error::Eval(
            "mutable hosted alias requires an explicit provider/operator epoch".into(),
        )),
    }
}

fn required(value: String, label: &str) -> Result<String> {
    (!value.trim().is_empty())
        .then_some(value)
        .ok_or_else(|| Error::Eval(format!("candidate identity requires {label}")))
}
fn artifact_text(value: &ArtifactEvidence) -> String {
    match value {
        ArtifactEvidence::Digest { algorithm, value } => format!("digest:{algorithm}:{value}"),
        ArtifactEvidence::Epoch(value) => format!("epoch:{value}"),
    }
}
fn field(name: &str, value: String) -> (sim_kernel::Symbol, Datum) {
    (sim_kernel::Symbol::new(name), Datum::String(value))
}
fn sorted_list_field(name: &str, values: &[String]) -> (sim_kernel::Symbol, Datum) {
    let mut values = values.to_vec();
    values.sort();
    (
        sim_kernel::Symbol::new(name),
        Datum::List(values.into_iter().map(Datum::String).collect()),
    )
}
fn option_number(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn expr_revision(value: &sim_kernel::Expr) -> Result<String> {
    Ok(Datum::try_from(value.clone())?.content_id()?.to_string())
}
