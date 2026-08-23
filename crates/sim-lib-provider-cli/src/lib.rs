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
        )
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

fn render_fan_out(inventory: &ProviderInventory, target: Option<&String>) -> Result<String> {
    let target = target.map(String::as_str).unwrap_or("all");
    let seats: Vec<_> = if target == "all" {
        inventory.seats.iter().collect()
    } else {
        vec![
            inventory
                .seats
                .iter()
                .find(|seat| seat.id == target)
                .ok_or_else(|| Error::Eval(format!("unknown provider seat {target}")))?,
        ]
    };
    Ok(seats.into_iter().map(|seat| format!(
        "operation=fan-out seat-id={} family=provider/{} status=planned mode=explicit provider-organ=sim-lib-provider\n",
        seat.id, seat.family
    )).collect())
}

fn read_inventory(path: &str) -> Result<ProviderInventory> {
    let source = fs::read_to_string(Path::new(path))
        .map_err(|err| Error::Eval(format!("read provider inventory {path}: {err}")))?;
    ProviderInventory::from_toml(&source)
}

fn split_inventory(args: &[String]) -> Result<(String, Vec<String>)> {
    let mut path = None;
    let mut words = Vec::new();
    let mut iter = args.iter().filter(|arg| arg.as_str() != "provider");
    while let Some(arg) = iter.next() {
        if arg == "--inventory" {
            path = iter.next().cloned();
        } else {
            words.push(arg.clone());
        }
    }
    Ok((
        path.ok_or_else(|| Error::Eval("provider requires --inventory <path>".into()))?,
        words,
    ))
}

fn render_families(inventory: &ProviderInventory) -> Result<String> {
    let mut families: Vec<_> = inventory
        .seats
        .iter()
        .map(|seat| seat.family.as_str())
        .collect();
    families.sort_unstable();
    families.dedup();
    Ok(families
        .into_iter()
        .map(|family| format!("family=provider/{family}\n"))
        .collect())
}

fn render_seats(inventory: &ProviderInventory) -> Result<String> {
    inventory.seats.iter().map(render_seat).collect()
}

fn render_show(inventory: &ProviderInventory, id: Option<&String>) -> Result<String> {
    let id = id.ok_or_else(|| Error::Eval("provider show requires a seat id".into()))?;
    let seat = inventory
        .seats
        .iter()
        .find(|seat| &seat.id == id)
        .ok_or_else(|| Error::Eval(format!("unknown provider seat {id}")))?;
    render_seat(seat)
}

fn render_status(inventory: &ProviderInventory) -> Result<String> {
    inventory
        .census(&[])?
        .into_iter()
        .map(|row| {
            Ok(format!(
                "seat-id={} family=provider/{} readiness={} reason={} next-action={}\n",
                row.seat_id,
                row.family,
                state_name(row.state),
                row.reason,
                row.next_action
            ))
        })
        .collect()
}

fn render_seat(seat: &ProviderSeatConfig) -> Result<String> {
    let transport = if seat.config_home_ref.is_some() {
        "broker-process"
    } else if matches!(seat.family.as_str(), "ollama" | "lemonade" | "lm-studio") {
        "local-http"
    } else {
        "https"
    };
    let semantics = if seat.config_home_ref.is_some() {
        "agent-task"
    } else {
        "model-turn"
    };
    let identity = seat
        .config_home_ref
        .as_deref()
        .unwrap_or(&seat.endpoint_label);
    let wire = match seat.family.as_str() {
        "anthropic-api" | "claude-cli" => "anthropic",
        "ollama" => "ollama",
        _ => "openai-compatible",
    };
    let terms = seat
        .terms_acknowledgement
        .as_deref()
        .unwrap_or("not-required");
    Ok(format!(
        "seat-id={} family=provider/{} transport={} semantics={} principal-kind={} identity={} wire={} readiness=cached-unknown terms={}\n",
        seat.id, seat.family, transport, semantics, seat.auth, identity, wire, terms
    ))
}

fn state_name(state: CensusState) -> &'static str {
    match state {
        CensusState::Ready => "ready",
        CensusState::Unavailable => "unavailable",
        CensusState::Expired => "expired",
        CensusState::Refused => "refused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_subscription_cli_and_local_daemon_are_distinct_command_rows() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/provider-seats.toml");
        let output = run(&[
            "provider".into(),
            "--inventory".into(),
            path.display().to_string(),
            "seats".into(),
        ])
        .unwrap();
        assert_eq!(output.lines().count(), 3);
        for id in [
            "openai-api-primary",
            "codex-subscription-primary",
            "ollama-local",
        ] {
            assert!(output.contains(&format!("seat-id={id} ")));
        }
        assert!(output.contains("principal-kind=api-key"));
        assert!(output.contains("principal-kind=subscription"));
        assert!(output.contains("principal-kind=none"));
        let fanout = run(&[
            "provider".into(),
            "--inventory".into(),
            path.display().to_string(),
            "fan-out".into(),
            "all".into(),
        ])
        .unwrap();
        assert_eq!(fanout.lines().count(), 3);
    }
}
