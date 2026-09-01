use crate::{AuthMetadata, ProviderSeatId, auth_metadata_key};
use sim_citizen::{CitizenField, field_error};
use sim_citizen_derive::Citizen;
use sim_kernel::{Expr, Result, Symbol};

/// Open metadata for one provider implementation family.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "provider/ProviderFamilyCard", version = 1)]
pub struct ProviderFamilyCard {
    /// Family identity, conventionally `provider/<family-name>`.
    pub family: Symbol,
    /// Transport category such as `http` or `broker-process`.
    pub transport: Symbol,
    /// Semantic category such as `model-turn` or `agent-task`.
    pub semantics: Symbol,
    /// Owner of authentication state, such as `sim` or `vendor-cli`.
    pub auth_owner: Symbol,
    /// Supported wire dialects.
    pub wires: Vec<Symbol>,
    /// Supported setup and authentication operations.
    pub operations: Vec<Symbol>,
    /// Provider-defined revision data.
    pub revision: Expr,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

/// Redacted principal identity attached to a provider seat.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "provider/PrincipalCard", version = 1)]
pub struct PrincipalCard {
    /// Human-readable, non-secret principal label.
    pub label: String,
    /// Principal kind such as `api-key`, `oauth`, or `none`.
    pub kind: Symbol,
    /// Credential owner such as `secret-provider`, `broker-owned`, or `none`.
    pub source: Symbol,
    /// Stable redacted digest; never credential material.
    pub digest: String,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

/// Address and transport metadata for a provider seat endpoint.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "provider/EndpointCard", version = 1)]
pub struct EndpointCard {
    /// Redaction-safe endpoint address or logical name.
    pub address: String,
    /// Endpoint transport category.
    pub transport: Symbol,
    /// Provider-defined endpoint revision data.
    pub revision: Expr,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

/// Metadata for an optional provider-owned execution harness.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "provider/HarnessCard", version = 1)]
pub struct HarnessCard {
    /// Harness kind, such as `vendor-cli` or `broker-server`.
    pub kind: Symbol,
    /// Redaction-safe harness identity or executable label.
    pub label: String,
    /// Provider-defined harness revision data.
    pub revision: Expr,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

/// Optional advertised capacity limits for one provider seat.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "provider/ProviderSeatLimits", version = 1)]
pub struct ProviderSeatLimits {
    /// Maximum simultaneous calls, when declared.
    pub concurrency: Option<u32>,
    /// Maximum requests per minute, when declared.
    pub requests_per_minute: Option<u64>,
    /// Maximum tokens per minute, when declared.
    pub tokens_per_minute: Option<u64>,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

/// Open metadata for one independently selectable provider seat.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "provider/ProviderSeatCard", version = 1)]
pub struct ProviderSeatCard {
    /// Stable seat identity.
    pub seat: ProviderSeatId,
    /// Provider family identity.
    pub family: Symbol,
    /// Redacted principal metadata.
    pub principal: PrincipalCard,
    /// Endpoint metadata.
    pub endpoint: EndpointCard,
    /// Optional provider-owned harness metadata.
    pub harness: Option<HarnessCard>,
    /// Optional selected model name.
    pub model: Option<String>,
    /// Advertised capacity limits.
    pub limits: ProviderSeatLimits,
    /// Provider-defined seat revision data.
    pub revision: Expr,
    /// Open extension fields.
    pub extra: Vec<(Expr, Expr)>,
}

fn auth_metadata(extra: &[(Expr, Expr)]) -> Result<Option<AuthMetadata>> {
    let key = auth_metadata_key();
    extra
        .iter()
        .find_map(|(candidate, value)| (candidate == &key).then_some(value))
        .map(AuthMetadata::from_expr)
        .transpose()
}

impl ProviderFamilyCard {
    /// Returns whether typed authentication metadata is present on this card.
    pub fn auth_metadata(&self) -> Result<Option<AuthMetadata>> {
        auth_metadata(&self.extra)
    }

    /// Records typed authentication metadata as a redaction-safe card extension.
    pub fn set_auth_metadata(&mut self, metadata: &AuthMetadata) {
        set_auth_metadata(&mut self.extra, metadata);
    }
}

impl ProviderSeatCard {
    /// Returns whether typed authentication metadata is present on this card.
    pub fn auth_metadata(&self) -> Result<Option<AuthMetadata>> {
        auth_metadata(&self.extra)
    }

    /// Records typed authentication metadata as a redaction-safe card extension.
    pub fn set_auth_metadata(&mut self, metadata: &AuthMetadata) {
        set_auth_metadata(&mut self.extra, metadata);
    }
}

fn set_auth_metadata(extra: &mut Vec<(Expr, Expr)>, metadata: &AuthMetadata) {
    let key = auth_metadata_key();
    extra.retain(|(candidate, _)| candidate != &key);
    extra.push((key, metadata.to_expr()));
}

impl Default for ProviderFamilyCard {
    fn default() -> Self {
        Self {
            family: Symbol::qualified("provider", "fixture"),
            transport: Symbol::new("fixture"),
            semantics: Symbol::new("model-turn"),
            auth_owner: Symbol::new("none"),
            wires: Vec::new(),
            operations: Vec::new(),
            revision: Expr::Nil,
            extra: Vec::new(),
        }
    }
}

impl Default for PrincipalCard {
    fn default() -> Self {
        Self {
            label: "fixture".to_owned(),
            kind: Symbol::new("none"),
            source: Symbol::new("none"),
            digest: "redacted-fixture".to_owned(),
            extra: Vec::new(),
        }
    }
}

impl Default for EndpointCard {
    fn default() -> Self {
        Self {
            address: "fixture".to_owned(),
            transport: Symbol::new("fixture"),
            revision: Expr::Nil,
            extra: Vec::new(),
        }
    }
}

impl Default for HarnessCard {
    fn default() -> Self {
        Self {
            kind: Symbol::new("fixture"),
            label: "fixture".to_owned(),
            revision: Expr::Nil,
            extra: Vec::new(),
        }
    }
}

impl Default for ProviderSeatCard {
    fn default() -> Self {
        Self {
            seat: ProviderSeatId::default(),
            family: Symbol::qualified("provider", "fixture"),
            principal: PrincipalCard::default(),
            endpoint: EndpointCard::default(),
            harness: None,
            model: None,
            limits: ProviderSeatLimits::default(),
            revision: Expr::Nil,
            extra: Vec::new(),
        }
    }
}

macro_rules! citizen_field_list {
    ($type:ty, $expected:literal, [$($field:ident),+ $(,)?]) => {
        impl CitizenField for $type {
            fn encode_field(&self) -> Expr {
                Expr::List(vec![$(self.$field.encode_field()),+])
            }

            fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
                let Expr::List(items) = expr else {
                    return Err(field_error(field, concat!("expected ", $expected, " list")));
                };
                let [$($field),+] = items.as_slice() else {
                    return Err(field_error(field, concat!("wrong field count for ", $expected)));
                };
                Ok(Self {
                    $($field: CitizenField::decode_field_expr($field, field)?),+
                })
            }
        }
    };
}

citizen_field_list!(
    PrincipalCard,
    "principal card",
    [label, kind, source, digest, extra]
);
citizen_field_list!(
    EndpointCard,
    "endpoint card",
    [address, transport, revision, extra]
);
citizen_field_list!(HarnessCard, "harness card", [kind, label, revision, extra]);
citizen_field_list!(
    ProviderSeatLimits,
    "provider seat limits",
    [concurrency, requests_per_minute, tokens_per_minute, extra,]
);
