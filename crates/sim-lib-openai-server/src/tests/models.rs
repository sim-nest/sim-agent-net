use serde_json::Value;
use sim_kernel::Symbol;
use sim_lib_agent_runner_core::ModelCard;

use super::*;

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
