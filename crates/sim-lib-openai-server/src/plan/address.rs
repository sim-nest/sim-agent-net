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

/// Resolves an atom address into a [`BackendDescriptor`], erroring when the
/// backend head is unknown.
pub fn resolve_atom_address(address: &str) -> Result<BackendDescriptor> {
    let Some((head, _)) = address.split_once('/') else {
        return Err(model_not_found(address));
    };
    if !KNOWN_HEADS.contains(&head) {
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

const KNOWN_HEADS: &[&str] = &[
    "openai", "ollama", "process", "runner", "agent", "skill", "sim", "fixture", "gateway",
];
