use super::ProviderConfig;
use crate::provider_profiles;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Expr, NumberLiteral, Symbol, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[test]
fn parses_current_openai_compatible_option_map() {
    let mut cx = test_cx();
    let mut options = HashMap::new();
    insert(
        &mut cx,
        &mut options,
        "name",
        Expr::Symbol(Symbol::new("hosted")),
    );
    insert(
        &mut cx,
        &mut options,
        "model",
        Expr::String("model-a".to_owned()),
    );
    insert(
        &mut cx,
        &mut options,
        "endpoint",
        Expr::String("http://127.0.0.1:8080/v1".to_owned()),
    );
    insert(
        &mut cx,
        &mut options,
        "api-key-env",
        Expr::String("TEST_KEY".to_owned()),
    );
    insert(
        &mut cx,
        &mut options,
        "codec",
        Expr::Symbol(Symbol::qualified("codec", "openai")),
    );
    insert(
        &mut cx,
        &mut options,
        "timeout",
        Expr::String("750ms".to_owned()),
    );
    insert(&mut cx, &mut options, "stream", Expr::Bool(true));
    insert(&mut cx, &mut options, "tools", Expr::Bool(false));
    insert(&mut cx, &mut options, "max-output-bytes", number("4096"));

    let config =
        ProviderConfig::from_options(provider_profiles::openai_compatible(), &mut cx, &options)
            .unwrap();

    assert_eq!(config.runner, Symbol::new("hosted"));
    assert_eq!(config.codec, Symbol::qualified("codec", "openai"));
    assert_eq!(config.endpoint, "http://127.0.0.1:8080/v1");
    assert_eq!(config.model, "model-a");
    assert_eq!(config.api_key_env, Some("TEST_KEY".to_owned()));
    assert_eq!(config.locality, Symbol::new("network"));
    assert_eq!(config.timeout, Duration::from_millis(750));
    assert!(config.stream);
    assert!(!config.tools);
    assert_eq!(config.max_output_bytes, 4096);
}

#[test]
fn generic_profile_requires_endpoint() {
    let mut cx = test_cx();
    let error = ProviderConfig::from_options(
        provider_profiles::openai_compatible(),
        &mut cx,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires endpoint"));
}

#[test]
fn ollama_defaults_preserve_existing_constructor_behavior() {
    let mut cx = test_cx();
    let config =
        ProviderConfig::from_options(provider_profiles::ollama(), &mut cx, &HashMap::new())
            .unwrap();

    assert_eq!(config.runner, Symbol::qualified("runner", "ollama"));
    assert_eq!(config.codec, Symbol::qualified("codec", "ollama"));
    assert_eq!(config.endpoint, "http://127.0.0.1:11434");
    assert_eq!(config.model, "qwen3.5:4b");
    assert_eq!(config.api_key_env, None);
    assert_eq!(config.locality, Symbol::new("local"));
    assert_eq!(config.timeout, Duration::from_secs(120));
    assert!(config.stream);
    assert!(!config.tools);
    assert_eq!(config.max_output_bytes, 1024 * 1024);
}

#[test]
fn local_profile_becomes_network_when_endpoint_is_not_loopback() {
    let mut cx = test_cx();
    let mut options = HashMap::new();
    insert(
        &mut cx,
        &mut options,
        "endpoint",
        Expr::String("https://models.example/v1".to_owned()),
    );

    let config =
        ProviderConfig::from_options(provider_profiles::lm_studio(), &mut cx, &options).unwrap();

    assert_eq!(config.locality, Symbol::new("network"));
    assert_eq!(config.api_key_env, None);
}

#[test]
fn lm_studio_auth_env_is_optional_but_accepted() {
    let mut cx = test_cx();
    let mut options = HashMap::new();
    insert(
        &mut cx,
        &mut options,
        "api-key-env",
        Expr::String("LM_STUDIO_API_KEY".to_owned()),
    );

    let config =
        ProviderConfig::from_options(provider_profiles::lm_studio(), &mut cx, &options).unwrap();

    assert_eq!(config.api_key_env, Some("LM_STUDIO_API_KEY".to_owned()));
    assert_eq!(config.locality, Symbol::new("local"));
}

#[test]
fn nil_api_key_env_disables_profile_default_auth_env() {
    let mut cx = test_cx();
    let mut options = HashMap::new();
    insert(&mut cx, &mut options, "api-key-env", Expr::Nil);

    let config =
        ProviderConfig::from_options(provider_profiles::openai(), &mut cx, &options).unwrap();

    assert_eq!(config.api_key_env, None);
}

fn test_cx() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    )
}

fn insert(cx: &mut Cx, options: &mut HashMap<String, Value>, key: &str, expr: Expr) {
    options.insert(key.to_owned(), cx.factory().expr(expr).unwrap());
}

fn number(canonical: &str) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: canonical.to_owned(),
    })
}
