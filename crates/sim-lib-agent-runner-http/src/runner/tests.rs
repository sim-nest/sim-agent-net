use super::{HttpRunner, anthropic_headers};
use crate::{ProviderAuth, ProviderConfig, provider_profiles};
use sim_kernel::Symbol;
use sim_lib_agent_runner_core::ModelRunner;
use std::{collections::HashMap, time::Duration};

#[test]
fn new_provider_maps_config_onto_existing_runner_fields() {
    let profile = provider_profiles::anthropic();
    let config = ProviderConfig {
        profile: profile.clone(),
        runner: Symbol::new("claude"),
        codec: Symbol::qualified("codec", "anthropic"),
        endpoint: "https://api.anthropic.com/v1".to_owned(),
        model: "claude-sonnet-latest".to_owned(),
        api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
        locality: Symbol::new("network"),
        timeout: Duration::from_secs(45),
        stream: true,
        tools: true,
        max_output_bytes: 8192,
    };

    let runner = HttpRunner::new_provider(config);

    assert_eq!(runner.runner, Symbol::new("claude"));
    assert_eq!(runner.model, "claude-sonnet-latest");
    assert_eq!(runner.provider, Symbol::new("anthropic"));
    assert_eq!(runner.locality, Symbol::new("network"));
    assert_eq!(runner.runner_label, "runner/provider");
    assert_eq!(runner.request_path, "/messages");
    assert_eq!(runner.endpoint, "https://api.anthropic.com/v1");
    assert_eq!(runner.api_key_env, Some("ANTHROPIC_API_KEY".to_owned()));
    assert_eq!(
        runner.auth,
        ProviderAuth::HeaderEnv {
            header: "x-api-key".to_owned(),
            env: "ANTHROPIC_API_KEY".to_owned()
        }
    );
    assert_eq!(runner.codec, Symbol::qualified("codec", "anthropic"));
    assert_eq!(runner.timeout, Duration::from_secs(45));
    assert!(runner.stream);
    assert!(runner.tools);
    assert_eq!(runner.max_response_bytes, 8192);
    assert_eq!(profile.chat_path, "/messages");
}

#[test]
fn new_provider_card_uses_provider_and_locality() {
    let mut cx = sim_kernel::Cx::new(
        std::sync::Arc::new(sim_kernel::EagerPolicy),
        std::sync::Arc::new(sim_kernel::DefaultFactory),
    );
    let config =
        ProviderConfig::from_options(provider_profiles::ollama(), &mut cx, &HashMap::new())
            .unwrap();
    let card = HttpRunner::new_provider(config).card();

    assert_eq!(card.runner, Symbol::qualified("runner", "ollama"));
    assert_eq!(card.provider, Symbol::new("ollama"));
    assert_eq!(card.locality, Symbol::new("local"));
}

#[test]
fn anthropic_headers_include_secret_version_and_json_content_type() {
    assert_eq!(
        anthropic_headers("secret-token"),
        vec![
            ("x-api-key".to_owned(), "secret-token".to_owned()),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]
    );
}
