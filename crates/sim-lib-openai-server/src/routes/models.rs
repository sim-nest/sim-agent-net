use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value as SimValue};
use sim_lib_agent_runner_core::ModelCard;

use crate::server::{GatewayRequest, GatewayResponse, GatewayRouteState};

/// Route path for the OpenAI-shaped `GET /v1/models` discovery endpoint.
pub const MODELS_PATH: &str = "/v1/models";

const FIXTURE_ECHO_MODEL: &str = "fixture/echo";
const SIM_BROWSE_PLAN_MODEL: &str = "sim/browse/plan";

/// Represents a single entry in the model catalog, mapped to the OpenAI
/// `model` object shape (`id` plus `owned_by`).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenAiModel {
    id: String,
    owned_by: String,
    metadata: Map<String, Value>,
}

impl OpenAiModel {
    /// Builds a model entry from an id and the owning provider name.
    pub fn new(id: impl Into<String>, owned_by: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            owned_by: owned_by.into(),
            metadata: Map::new(),
        }
    }

    /// Returns the built-in `fixture/echo` model owned by `sim`.
    pub fn fixture_echo() -> Self {
        Self::new(FIXTURE_ECHO_MODEL, "sim")
    }

    /// Returns a SIM-native model with the given id owned by `sim`.
    pub fn sim_native(id: impl Into<String>) -> Self {
        Self::new(id, "sim")
    }

    /// Builds a model entry from a runner [`ModelCard`], taking the card's
    /// model id and provider.
    pub fn from_model_card(card: ModelCard) -> Self {
        let mut metadata = Map::new();
        metadata.insert("runner".to_owned(), Value::String(card.runner.to_string()));
        metadata.insert(
            "locality".to_owned(),
            Value::String(card.locality.to_string()),
        );
        for (key, value) in card.extra {
            metadata.insert(metadata_key(&key), metadata_value(&value));
        }
        Self {
            id: card.model,
            owned_by: card.provider.to_string(),
            metadata,
        }
    }

    /// Returns the model id.
    pub fn id(&self) -> &str {
        &self.id
    }

    fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("id".to_owned(), Value::String(self.id.clone()));
        object.insert("object".to_owned(), Value::String("model".to_owned()));
        object.insert("created".to_owned(), json!(0));
        object.insert("owned_by".to_owned(), Value::String(self.owned_by.clone()));
        if !self.metadata.is_empty() {
            object.insert("metadata".to_owned(), Value::Object(self.metadata.clone()));
        }
        Value::Object(object)
    }
}

/// Represents the deduplicated set of models advertised by `/v1/models`,
/// always including the built-in fixture and `sim/browse/plan` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCatalog {
    models: Vec<OpenAiModel>,
}

impl ModelCatalog {
    /// Returns the catalog containing only the built-in fixture models.
    pub fn default_fixture() -> Self {
        Self::from_parts(Vec::new())
    }

    /// Builds a catalog from runner model-card transcripts, parsing each
    /// expression into a [`ModelCard`] and failing on a malformed transcript.
    pub fn from_runner_card_exprs(cards: impl IntoIterator<Item = Expr>) -> Result<Self> {
        let cards = cards
            .into_iter()
            .map(ModelCard::try_from)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_model_cards(cards))
    }

    /// Builds a catalog from already-parsed runner model cards.
    pub fn from_model_cards(cards: impl IntoIterator<Item = ModelCard>) -> Self {
        let runner_models = cards
            .into_iter()
            .map(OpenAiModel::from_model_card)
            .collect::<Vec<_>>();
        Self::from_parts(runner_models)
    }

    /// Builds a catalog by calling the `runner/cards` function with the given
    /// arguments and parsing the returned list of model-card transcripts.
    ///
    /// Returns the fixture-only catalog when no arguments are supplied, and
    /// errors if `runner/cards` does not return a list.
    pub fn from_runner_args(cx: &mut Cx, args: Vec<SimValue>) -> Result<Self> {
        if args.is_empty() {
            return Ok(Self::default_fixture());
        }
        let cards = cx.call_function(&runner_cards_symbol(), Args::new(args))?;
        let expr = cards.object().as_expr(cx)?;
        let Expr::List(items) = expr else {
            return Err(Error::Eval(
                "runner/cards must return a list of model-card transcripts".to_owned(),
            ));
        };
        Self::from_runner_card_exprs(items)
    }

    /// Returns the catalog's models in advertised order.
    pub fn models(&self) -> &[OpenAiModel] {
        &self.models
    }

    fn from_parts(runner_models: Vec<OpenAiModel>) -> Self {
        let mut seen = BTreeSet::new();
        let mut models = Vec::new();
        push_unique(&mut models, &mut seen, OpenAiModel::fixture_echo());
        for model in runner_models {
            push_unique(&mut models, &mut seen, model);
        }
        push_unique(
            &mut models,
            &mut seen,
            OpenAiModel::sim_native(SIM_BROWSE_PLAN_MODEL),
        );
        Self { models }
    }

    fn to_json(&self) -> Value {
        json!({
            "object": "list",
            "data": self.models.iter().map(OpenAiModel::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Handles `GET /v1/models`, returning the catalog built from the registered
/// runner model cards.
pub fn handle_models(_request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    models_response_for_catalog(&ModelCatalog::from_model_cards(state.runners().cards()))
}

/// Returns a `/v1/models` response for the fixture-only catalog.
pub fn models_response() -> GatewayResponse {
    models_response_for_catalog(&ModelCatalog::default_fixture())
}

/// Returns a `/v1/models` response for the catalog produced by calling
/// `runner/cards` with the given arguments.
pub fn models_response_for_runner_args(
    cx: &mut Cx,
    args: Vec<SimValue>,
) -> Result<GatewayResponse> {
    let catalog = ModelCatalog::from_runner_args(cx, args)?;
    Ok(models_response_for_catalog(&catalog))
}

/// Encodes the given catalog as the OpenAI `list`-of-models JSON body.
pub fn models_response_for_catalog(catalog: &ModelCatalog) -> GatewayResponse {
    GatewayResponse::json_value(200, catalog.to_json())
}

/// Returns the `runner/cards` function symbol that fetches model cards.
pub fn runner_cards_symbol() -> Symbol {
    Symbol::qualified("runner", "cards")
}

fn push_unique(models: &mut Vec<OpenAiModel>, seen: &mut BTreeSet<String>, model: OpenAiModel) {
    if seen.insert(model.id.clone()) {
        models.push(model);
    }
}

fn metadata_key(expr: &Expr) -> String {
    match expr {
        Expr::Symbol(symbol) | Expr::Local(symbol) if symbol.namespace.is_none() => {
            normalize_metadata_key(symbol.name.as_ref())
        }
        Expr::Symbol(symbol) | Expr::Local(symbol) => normalize_metadata_key(&symbol.to_string()),
        Expr::String(value) => normalize_metadata_key(value),
        other => normalize_metadata_key(&format!("{other:?}")),
    }
}

fn normalize_metadata_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn metadata_value(expr: &Expr) -> Value {
    match expr {
        Expr::Nil => Value::Null,
        Expr::Bool(value) => Value::Bool(*value),
        Expr::Number(value) => number_json(&value.canonical),
        Expr::Symbol(symbol) | Expr::Local(symbol) if symbol.namespace.is_none() => {
            Value::String(symbol.name.as_ref().to_owned())
        }
        Expr::Symbol(symbol) | Expr::Local(symbol) => Value::String(symbol.to_string()),
        Expr::String(value) => Value::String(value.clone()),
        Expr::Bytes(bytes) => Value::Array(bytes.iter().map(|byte| json!(byte)).collect()),
        Expr::List(values) | Expr::Vector(values) | Expr::Set(values) | Expr::Block(values) => {
            Value::Array(values.iter().map(metadata_value).collect())
        }
        Expr::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(metadata_key(key), metadata_value(value));
            }
            Value::Object(object)
        }
        other => Value::String(format!("{other:?}")),
    }
}

fn number_json(canonical: &str) -> Value {
    serde_json::from_str(canonical).unwrap_or_else(|_| Value::String(canonical.to_owned()))
}
