//! Loadable `sim provider` command surface.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{fs, path::Path, sync::Arc};

use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Lib, LibManifest, LibTarget, Linker, LoadCx,
    Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_lib_provider::{CensusState, ProviderInventory, ProviderSeatConfig};
use sim_run_core::{cli_envelope_args, cli_main_entrypoint_symbol};

const VERBS: &[&str] = &[
    "families",
    "seats",
    "show",
    "discover",
    "probe",
    "open-check",
    "auth-methods",
    "login",
    "status",
    "logout",
    "fan-out",
];

/// Host-registered provider command library.
#[derive(Clone, Default)]
pub struct ProviderCommandLib;

impl ProviderCommandLib {
    /// Builds the provider command library.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Lib for ProviderCommandLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "provider-cli"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: provider_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            provider_entrypoint_symbol(),
            cx.factory().opaque(Arc::new(ProviderEntrypoint))?,
        )?;
        Ok(())
    }
}

/// Symbol exported for the bootloader's `provider` verb.
#[must_use]
pub fn provider_entrypoint_symbol() -> Symbol {
    cli_main_entrypoint_symbol("provider")
}

#[derive(Clone)]
struct ProviderEntrypoint;
impl Object for ProviderEntrypoint {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("cli/main/provider".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for ProviderEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for ProviderEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing provider envelope".into()))?;
        let argv = cli_envelope_args(cx, envelope)?;
        print!("{}", run(&argv)?);
        cx.factory().bool(true)
    }
}

/// Executes a provider command against an explicitly supplied inventory.
pub fn run(args: &[String]) -> Result<String> {
    let (inventory_path, words) = split_inventory(args)?;
    let verb = words.first().map(String::as_str).unwrap_or("seats");
    if !VERBS.contains(&verb) {
        return Err(Error::Eval(format!("unknown provider verb {verb}")));
    }
    let inventory = read_inventory(&inventory_path)?;
    match verb {
        "families" => render_families(&inventory),
        "seats" => render_seats(&inventory),
        "show" => render_show(&inventory, words.get(1)),
        "status" => render_status(&inventory),
        "auth-methods" => {
            Ok("api-key\noauth-browser\noauth-device\nsubscription\nbroker-owned\nnone\n".into())
        }
        // These are deliberately explicit operator operations. The adapter does
        // not guess a vendor or perform ambient discovery/authentication.
        "fan-out" => render_fan_out(&inventory, words.get(1)),
        "discover" | "probe" | "open-check" | "login" | "logout" => {
            let seat = words.get(1).map(String::as_str).unwrap_or("all");
            Ok(format!(
                "operation={verb} target={seat} mode=explicit provider-organ=sim-lib-provider\n"
            ))
        }
        _ => unreachable!(),
    }
}

include!("render.rs");

#[cfg(test)]
mod tests;
