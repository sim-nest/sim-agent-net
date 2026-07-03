use sim_kernel::{Cx, Error, Result, Symbol};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{EvalSite, LineDriver, ServerAddress};

/// Outcome of resolving a [`ServerAddress`] to a concrete eval site and codec set.
pub struct ResolvedAddress {
    /// Eval site that handles frames for the resolved address.
    pub site: Arc<dyn EvalSite>,
    /// Codec selected for the connection.
    pub selected_codec: Symbol,
    /// Codecs the resolved site supports.
    pub supported_codecs: Vec<Symbol>,
}

/// Function that resolves a [`ServerAddress`] (with offered codecs) to a [`ResolvedAddress`].
pub type AddressResolver =
    fn(&mut Cx, &ServerAddress, &[Symbol]) -> Result<Option<ResolvedAddress>>;
/// Function that builds a [`LineDriver`] from a configuration expression.
pub type LineDriverFactory = fn(&mut Cx, &sim_kernel::Expr) -> Result<Option<Box<dyn LineDriver>>>;

pub(crate) fn address_resolvers() -> &'static Mutex<BTreeMap<String, AddressResolver>> {
    static RESOLVERS: OnceLock<Mutex<BTreeMap<String, AddressResolver>>> = OnceLock::new();
    RESOLVERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn line_driver_factories() -> &'static Mutex<BTreeMap<String, LineDriverFactory>> {
    static FACTORIES: OnceLock<Mutex<BTreeMap<String, LineDriverFactory>>> = OnceLock::new();
    FACTORIES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Registers an [`AddressResolver`] under an address `kind` in the global resolver registry.
pub fn register_address_resolver(kind: Symbol, resolver: AddressResolver) -> Result<()> {
    let mut resolvers = address_resolvers()
        .lock()
        .map_err(|_| Error::HostError("address resolver registry mutex poisoned".to_owned()))?;
    resolvers.insert(kind.to_string(), resolver);
    Ok(())
}

/// Registers a [`LineDriverFactory`] under `name` in the global line-driver registry.
pub fn register_line_driver(name: Symbol, factory: LineDriverFactory) -> Result<()> {
    let mut factories = line_driver_factories()
        .lock()
        .map_err(|_| Error::HostError("line driver registry mutex poisoned".to_owned()))?;
    factories.insert(name.to_string(), factory);
    Ok(())
}
