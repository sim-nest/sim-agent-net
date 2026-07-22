use sim_kernel::{Error, Result, Symbol};

/// Resolved description of a plan atom address, identifying the backend that
/// should serve it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendDescriptor {
    /// Full atom address (for example `openai/gpt-4o`).
    pub address: String,
    /// Leading address segment naming the backend family (for example `openai`).
    pub head: String,
    /// Runner symbol derived from the full address.
    pub runner: Symbol,
    /// Whether the address designates a built-in fixture backend.
    pub fixture: bool,
}

impl BackendDescriptor {
    /// Returns `true` when this atom should dispatch through a registered runner.
    pub fn is_runner_backed(&self) -> bool {
        !self.fixture && self.head != "gateway"
    }

    /// Returns `true` when this atom should dispatch through gateway federation.
    pub fn is_gateway(&self) -> bool {
        self.head == "gateway"
    }
}

/// Resolves an atom address into a [`BackendDescriptor`], erroring when the
/// backend head is unknown.
pub fn resolve_atom_address(address: &str) -> Result<BackendDescriptor> {
    let Some((head, _)) = address.split_once('/') else {
        return Err(model_not_found(address));
    };
    if !KNOWN_PROVIDER_PREFIXES.contains(&head) {
        return Err(model_not_found(address));
    }
    Ok(BackendDescriptor {
        address: address.to_owned(),
        head: head.to_owned(),
        runner: Symbol::new(address.to_owned()),
        fixture: head == "fixture",
    })
}

fn model_not_found(address: &str) -> Error {
    Error::Eval(format!("model_not_found: {address}"))
}

const KNOWN_PROVIDER_PREFIXES: &[&str] = &[
    "openai",
    "anthropic",
    "ollama",
    "lm-studio",
    "lemonade",
    "process",
    "runner",
    "agent",
    "skill",
    "sim",
    "fixture",
    "gateway",
];

/// Returns the open set of accepted gateway plan address prefixes.
pub fn provider_prefixes() -> &'static [&'static str] {
    KNOWN_PROVIDER_PREFIXES
}
