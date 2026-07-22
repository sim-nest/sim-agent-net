use std::sync::Arc;

use serde_json::{Value, json};
use sim_kernel::{Cx, Expr, Symbol};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};

use super::*;
use crate::routes::models::handle_models;

#[test]
fn models_catalog_lists_loadable_local_model_card() {
    let catalog = ModelCatalog::from_model_cards(vec![ModelCard::new(
        Symbol::new("runner/local-model"),
        "sim-local-stub",
        Symbol::new("local-model"),
        Symbol::new("local"),
    )]);
    let response = models_response_for_catalog(&catalog);
    let json: Value = serde_json::from_slice(response.body()).unwrap();
    let local = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "sim-local-stub")
        .expect("local model card appears in /v1/models catalog");

    assert_eq!(local["owned_by"], "local-model");
}

#[test]
fn models_catalog_projects_native_card_metadata() {
    let mut card = ModelCard::new(
        Symbol::new("runner/lm-studio"),
        "lm-studio/local-default",
        Symbol::new("lm-studio"),
        Symbol::new("local"),
    );
    card.extra.push((
        Expr::Symbol(Symbol::new("probe-status")),
        Expr::Symbol(Symbol::new("available")),
    ));
    card.extra.push((
        Expr::Symbol(Symbol::new("modalities-in")),
        Expr::List(vec![
            Expr::Symbol(Symbol::new("text")),
            Expr::Symbol(Symbol::new("image")),
        ]),
    ));

    let registry = OpenAiRunnerRegistry::new()
        .with_runner("lm-studio/local-default", Arc::new(CardOnlyRunner { card }));
    let state = GatewayRouteState::memory().with_runners(registry);
    let response = handle_models(&GatewayRequest::get(MODELS_PATH), &state);
    let json: Value = serde_json::from_slice(response.body()).unwrap();
    let local = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "lm-studio/local-default")
        .expect("native card appears in /v1/models catalog");

    assert_eq!(local["owned_by"], "lm-studio");
    assert_eq!(local["metadata"]["runner"], "runner/lm-studio");
    assert_eq!(local["metadata"]["locality"], "local");
    assert_eq!(local["metadata"]["probe_status"], "available");
    assert_eq!(local["metadata"]["modalities_in"], json!(["text", "image"]));
}

#[derive(Clone)]
struct CardOnlyRunner {
    card: ModelCard,
}

impl ModelRunner for CardOnlyRunner {
    fn card(&self) -> ModelCard {
        self.card.clone()
    }

    fn infer(&self, _cx: &mut Cx, _request: ModelRequest) -> sim_kernel::Result<ModelResponse> {
        Ok(ModelResponse::default())
    }
}
