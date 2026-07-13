use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Export, Lib, LibManifest, LibTarget, Linker, Result, Symbol, Version,
};

use crate::codec_openai::{install_openai_codec, openai_codec_symbol};
use crate::ops::{
    OpenAiGatewayFunction, cache_stats_symbol, capability_report_symbol, events_symbol,
    fabric_symbol, health_symbol, key_add_symbol, key_list_symbol, model_health_symbol,
    models_symbol, plan_check_symbol, plan_combinators_symbol, plan_explain_symbol,
    plan_parse_symbol, plan_run_symbol, run_get_symbol, runs_symbol, serve_symbol,
    storage_stats_symbol,
};

const OPENAI_GATEWAY_LIB_ID: &str = "openai-gateway";

/// Loadable library that installs the OpenAI-compatible gateway surface.
///
/// Implements [`Lib`]: its manifest declares the gateway's id, version, ABI,
/// and exports, and its loader registers the OpenAI codec plus every
/// [`OpenAiGatewayFunction`] under its operation symbol.
pub struct OpenAiGatewayLib;

impl Lib for OpenAiGatewayLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: manifest_name(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: openai_gateway_exports(),
        }
    }

    fn load(&self, cx: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        install_openai_codec(cx, linker)?;
        for function in [
            OpenAiGatewayFunction::serve(),
            OpenAiGatewayFunction::health(),
            OpenAiGatewayFunction::models(),
            OpenAiGatewayFunction::plan_parse(),
            OpenAiGatewayFunction::plan_check(),
            OpenAiGatewayFunction::plan_run(),
            OpenAiGatewayFunction::plan_explain(),
            OpenAiGatewayFunction::plan_combinators(),
            OpenAiGatewayFunction::fabric(),
            OpenAiGatewayFunction::key_add(),
            OpenAiGatewayFunction::key_list(),
            OpenAiGatewayFunction::runs(),
            OpenAiGatewayFunction::run_get(),
            OpenAiGatewayFunction::events(),
            OpenAiGatewayFunction::storage_stats(),
            OpenAiGatewayFunction::model_health(),
            OpenAiGatewayFunction::cache_stats(),
            OpenAiGatewayFunction::capability_report(),
        ] {
            linker.function_value(function.symbol(), cx.factory().opaque(Arc::new(function))?)?;
        }
        Ok(())
    }
}

/// Returns the library's manifest id symbol, `sim/openai-gateway`.
pub fn manifest_name() -> Symbol {
    Symbol::qualified("sim", OPENAI_GATEWAY_LIB_ID)
}

/// Returns the full export list for the gateway library: every operation
/// function symbol plus the OpenAI codec.
pub fn openai_gateway_exports() -> Vec<Export> {
    let mut exports = [
        serve_symbol(),
        health_symbol(),
        models_symbol(),
        plan_parse_symbol(),
        plan_check_symbol(),
        plan_run_symbol(),
        plan_explain_symbol(),
        plan_combinators_symbol(),
        fabric_symbol(),
        key_add_symbol(),
        key_list_symbol(),
        runs_symbol(),
        run_get_symbol(),
        events_symbol(),
        storage_stats_symbol(),
        model_health_symbol(),
        cache_stats_symbol(),
        capability_report_symbol(),
    ]
    .into_iter()
    .map(|symbol| Export::Function {
        symbol,
        function_id: None,
    })
    .collect::<Vec<_>>();
    exports.push(Export::Codec {
        symbol: openai_codec_symbol(),
        codec_id: None,
    });
    exports
}
