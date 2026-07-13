use sim_config::{ConfigProbe, ConfigProbeCaps, ConfigProbeRequest, ConfigProbeStatus, ProbeMode};
use sim_kernel::{Args, Expr, Symbol};
use sim_lib_agent_runner_core::ModelCard;

use super::support::{eval_cx, install_agent_lib, install_test_codec};
use crate::{AgentModelConfigProbe, AgentModelProviderPresence, model_defaults_config_lib_symbol};

#[test]
fn config_probe_modeled_emits_fixture_defaults_without_environment() {
    let probe = AgentModelConfigProbe::modeled();
    let request = modeled_request();

    let (layer, report) = probe.probe(&request);

    assert_eq!(report.status, ConfigProbeStatus::Applied);
    assert_eq!(
        report.emitted_keys,
        [
            "model_regex",
            "provider_regex",
            "prefer_local",
            "default_model",
            "openai_key_present",
            "openai_base_present",
            "ollama_host_present",
        ]
    );
    let table = layer_table(layer.unwrap());
    assert_eq!(
        field(&table, "default_model"),
        &Expr::String("fixture/echo".to_owned())
    );
    assert_eq!(
        field(&table, "provider_regex"),
        &Expr::String(r"^(?:modeled)$".to_owned())
    );
    assert_eq!(field(&table, "prefer_local"), &Expr::Bool(true));
    assert_eq!(field(&table, "openai_key_present"), &Expr::Bool(false));
    assert_eq!(field(&table, "openai_base_present"), &Expr::Bool(false));
    assert_eq!(field(&table, "ollama_host_present"), &Expr::Bool(false));
}

#[test]
fn config_probe_real_requires_env_capability() {
    let probe = AgentModelConfigProbe::new(Vec::new(), AgentModelProviderPresence::default());
    let request = ConfigProbeRequest {
        mode: ProbeMode::Real,
        ..modeled_request()
    };

    let (layer, report) = probe.probe(&request);

    assert!(layer.is_none());
    assert_eq!(
        report.status,
        ConfigProbeStatus::Denied {
            capability: "env".to_owned()
        }
    );
    assert!(report.emitted_keys.is_empty());
}

#[test]
fn config_probe_real_selects_local_matching_card_deterministically() {
    let cards = vec![
        card(
            "runner/remote",
            "gpt-4.1-mini",
            "openai-compatible",
            "remote",
        ),
        card("runner/local", "sim/local-echo", "local-model", "local"),
        card("runner/other", "other/model", "other-provider", "local"),
    ];
    let probe = AgentModelConfigProbe::new(
        cards,
        AgentModelProviderPresence {
            openai_key_present: true,
            openai_base_present: true,
            ollama_host_present: false,
        },
    );
    let request = ConfigProbeRequest {
        mode: ProbeMode::Real,
        caps: ConfigProbeCaps {
            env: true,
            ..ConfigProbeCaps::default()
        },
        ..modeled_request()
    };

    let (layer, report) = probe.probe(&request);

    assert_eq!(report.status, ConfigProbeStatus::Applied);
    let table = layer_table(layer.unwrap());
    assert_eq!(
        field(&table, "default_model"),
        &Expr::String("sim/local-echo".to_owned())
    );
    assert_eq!(
        field(&table, "provider_regex"),
        &Expr::String(
            r"^(?:modeled|openai|openai-compatible|local-model|other-provider)$".to_owned()
        )
    );
    assert_eq!(field(&table, "openai_key_present"), &Expr::Bool(true));
    assert_eq!(field(&table, "openai_base_present"), &Expr::Bool(true));
    assert_eq!(field(&table, "ollama_host_present"), &Expr::Bool(false));
}

#[test]
fn config_probe_real_falls_back_when_model_regex_has_no_match() {
    let probe = AgentModelConfigProbe::new(
        vec![card(
            "runner/custom",
            "custom/model",
            "custom-provider",
            "remote",
        )],
        AgentModelProviderPresence {
            ollama_host_present: true,
            ..AgentModelProviderPresence::default()
        },
    );
    let request = ConfigProbeRequest {
        mode: ProbeMode::Real,
        caps: ConfigProbeCaps {
            env: true,
            ..ConfigProbeCaps::default()
        },
        ..modeled_request()
    };

    let (layer, report) = probe.probe(&request);

    assert_eq!(report.status, ConfigProbeStatus::Applied);
    let table = layer_table(layer.unwrap());
    assert_eq!(
        field(&table, "default_model"),
        &Expr::String("fixture/echo".to_owned())
    );
    assert_eq!(
        field(&table, "provider_regex"),
        &Expr::String(r"^(?:modeled|ollama|custom-provider)$".to_owned())
    );
}

#[test]
fn config_probe_skips_other_libs() {
    let probe = AgentModelConfigProbe::modeled();
    let request = ConfigProbeRequest {
        lib: Symbol::qualified("stream", "host"),
        ..modeled_request()
    };

    let (layer, report) = probe.probe(&request);

    assert!(layer.is_none());
    assert!(matches!(report.status, ConfigProbeStatus::Skipped { .. }));
}

#[test]
fn config_probe_defaults_can_drive_model_placement() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "echo"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory()
                    .symbol(Symbol::new("configured-local"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory()
                    .string("sim/configured-local".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let runner_list = cx.factory().list(vec![runner.clone()]).unwrap();
    let cards = cx
        .call_function(
            &Symbol::qualified("runner", "cards"),
            Args::new(vec![runner_list]),
        )
        .unwrap();
    let Expr::List(cards) = cards.object().as_expr(&mut cx).unwrap() else {
        panic!("runner/cards must return a list");
    };
    let cards = cards
        .into_iter()
        .map(ModelCard::try_from)
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();
    let probe = AgentModelConfigProbe::new(cards, AgentModelProviderPresence::default());
    let request = ConfigProbeRequest {
        mode: ProbeMode::Real,
        caps: ConfigProbeCaps {
            env: true,
            ..ConfigProbeCaps::default()
        },
        ..modeled_request()
    };

    let (layer, _) = probe.probe(&request);
    let table = layer_table(layer.unwrap());
    assert_eq!(
        field(&table, "default_model"),
        &Expr::String("sim/configured-local".to_owned())
    );

    let placed = cx
        .call_function(
            &Symbol::qualified("runner", "place"),
            Args::new(vec![
                cx.factory()
                    .string("model-site:configured-local".to_owned())
                    .unwrap(),
                runner,
            ]),
        )
        .unwrap();
    let placed = placed.object().as_expr(&mut cx).unwrap();
    assert_eq!(
        maybe_field(&placed, "model"),
        Some(&Expr::String("sim/configured-local".to_owned()))
    );
}

fn modeled_request() -> ConfigProbeRequest {
    ConfigProbeRequest {
        lib: model_defaults_config_lib_symbol(),
        mode: ProbeMode::Modeled,
        caps: ConfigProbeCaps::default(),
    }
}

fn card(runner: &str, model: &str, provider: &str, locality: &str) -> ModelCard {
    ModelCard::new(
        symbol(runner),
        model,
        Symbol::new(provider),
        Symbol::new(locality),
    )
}

fn symbol(value: &str) -> Symbol {
    match value.split_once('/') {
        Some((namespace, name)) => Symbol::qualified(namespace, name),
        None => Symbol::new(value),
    }
}

fn layer_table(layer: sim_config::ConfigLayer) -> Expr {
    layer
        .dir
        .table(&model_defaults_config_lib_symbol())
        .unwrap()
        .table
        .clone()
}

fn field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
    maybe_field(expr, name).unwrap_or_else(|| panic!("missing field {name}"))
}

fn maybe_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        panic!("expected map");
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.name.as_ref() == name => Some(value),
        _ => None,
    })
}
