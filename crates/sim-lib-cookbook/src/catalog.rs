//! Requires-driven library loading and the cookbook capability profile.
//!
//! COOKBOOK_7 turns the cookbook into a capability-scoped `realize` site with
//! requires-driven loading. Two contracts live here:
//!
//! - [`LibCatalog`]: a resolver from a recipe `requires` name to a loadable
//!   library. The catalog is assembled at the HOST/facade level (batteries `sim`,
//!   `sim-web-shell`) from the standard distribution, so NO dependency cycle
//!   touches the thin cookbook engine (R6/R7): the runner (`run.rs`) depends only
//!   on this trait, never on a concrete domain lib.
//! - [`CookbookCapabilityProfile`]: the deterministic capability set the cookbook
//!   seats its eval `Cx` with. It GRANTS pure/offline/deterministic effects,
//!   including local placement through `EvalFabric`, and DENIES live-net,
//!   live-hardware, process-spawn, wall-clock, fs/write, and unseeded entropy. An
//!   op that requests a denied capability fails closed (the capability is simply
//!   never granted), which is what makes a Category D recipe a descriptor BY
//!   CONSTRUCTION.

use sim_cookbook::RecipeCard;
use sim_kernel::{CapabilityName, Cx, GrantSeat, Lib, Result, Symbol};

macro_rules! grant_into_result {
    ($grant:expr) => {{
        #[allow(clippy::let_unit_value)]
        let grant_result = $grant;
        #[allow(clippy::unit_arg)]
        grant_result.into_result()
    }};
}

/// A resolver from a recipe `requires` name to a loadable library.
///
/// A recipe's `requires = [...]` list names the libs its setup needs
/// (`numbers/complex`, `codec/lisp`, `organ/binding`, ...). The runner asks the
/// catalog to resolve each name and loads the returned lib into the eval `Cx`
/// before decode+eval. A name the catalog does not carry stays unresolved, and
/// the recipe becomes a descriptor whose reason is the unresolved require.
pub trait LibCatalog {
    /// Resolve a cookbook require name to a loadable library, or `None` when this
    /// catalog does not carry it.
    ///
    /// A require matches either the lib's fully qualified cookbook name
    /// (`numbers/cas`) or its unqualified tail (`cas`), mirroring
    /// [`crate::missing_requires`].
    fn resolve(&self, name: &str) -> Option<&dyn Lib>;
}

/// The empty catalog: resolves nothing.
///
/// Callers on the compatibility path pre-load every required lib into the `Cx`
/// themselves; the runner then finds each require already present and loads
/// nothing. [`crate::run_recipe`] uses this so its behavior is unchanged.
pub struct EmptyCatalog;

impl LibCatalog for EmptyCatalog {
    fn resolve(&self, _name: &str) -> Option<&dyn Lib> {
        None
    }
}

/// Load every `requires` entry of `card` into `cx` via `catalog`, idempotently,
/// returning the names that stayed unresolved (neither already loaded nor
/// carried by the catalog).
///
/// A require already satisfied by a loaded lib is skipped. A require the catalog
/// resolves is loaded only if its id is not already registered (idempotent, the
/// `install_once` contract). An unresolved require is returned so the caller can
/// report the recipe as a descriptor.
pub fn load_requires(cx: &mut Cx, catalog: &dyn LibCatalog, card: &RecipeCard) -> Vec<String> {
    let mut unresolved = Vec::new();
    for req in &card.requires {
        if lib_present(cx, req) {
            continue;
        }
        match catalog.resolve(req) {
            Some(lib) => {
                let mut visiting = Vec::new();
                if let Err(err) = load_lib_with_deps(cx, catalog, lib, &mut visiting) {
                    unresolved.push(format!("{req} (load failed: {err})"));
                }
            }
            None => unresolved.push(req.clone()),
        }
    }
    unresolved
}

/// Whether a lib matching `name` (by qualified id or unqualified tail) is loaded.
fn lib_present(cx: &Cx, name: &str) -> bool {
    cx.registry().libs().iter().any(|lib| {
        lib.manifest.id.as_qualified_str() == name || lib.manifest.id.name.as_ref() == name
    })
}

/// Load `lib` into `cx` after its catalog-resolvable manifest dependencies, so a
/// lib whose `load` observes already-registered libs (e.g. a promotion rule over
/// existing number domains) sees them first regardless of the recipe's
/// `requires` order (COOK8.07 dependency-order loading). Idempotent (a
/// registered lib is skipped) and cycle-safe (an in-progress id is not
/// re-entered; R7 keeps the lib graph acyclic anyway).
fn load_lib_with_deps(
    cx: &mut Cx,
    catalog: &dyn LibCatalog,
    lib: &dyn Lib,
    visiting: &mut Vec<Symbol>,
) -> Result<()> {
    let manifest = lib.manifest();
    if cx.registry().lib(&manifest.id).is_some() || visiting.contains(&manifest.id) {
        return Ok(());
    }
    visiting.push(manifest.id.clone());
    for dep in &manifest.requires {
        let resolved = catalog
            .resolve(&dep.id.as_qualified_str())
            .or_else(|| catalog.resolve(dep.id.name.as_ref()));
        // A dependency the catalog does not carry is assumed already loaded (the
        // boot prelude) or genuinely absent; either way the lib's own load
        // surfaces the error. Only catalog-resolvable deps are ordered here.
        if let Some(dep_lib) = resolved {
            load_lib_with_deps(cx, catalog, dep_lib, visiting)?;
        }
    }
    cx.load_lib(lib)?;
    visiting.pop();
    Ok(())
}

/// The deterministic capability profile the cookbook seats its eval `Cx` with.
///
/// GRANTS the pure/offline/deterministic effects a recipe may legitimately need,
/// including eval-time macro expansion for recipes that load macro-capable
/// libraries. It DENIES (by omission) every live/effectful capability. A recipe
/// requesting a denied capability fails closed: it is a Category D descriptor
/// whose purpose is the denial. The profile is data, not a closed enum -- the
/// granted and denied vocabularies are named capability strings the kernel
/// interns.
#[derive(Clone, Debug, Default)]
pub struct CookbookCapabilityProfile;

impl CookbookCapabilityProfile {
    /// The capabilities this profile GRANTS: read-construct, read-eval,
    /// eval-time macro expansion, and the pure/offline/deterministic effect
    /// vocabulary (local eval-fabric placement, compute, codec round-trip,
    /// offline render with no device, deterministic cassette replay, and a
    /// deterministic model fixture).
    pub fn granted() -> Vec<CapabilityName> {
        [
            "read-construct",
            "read-eval",
            "macro.expand",
            "macro.expand.eval",
            "eval.fabric",
            "compute",
            "codec-encode",
            "codec-decode",
            "offline-render",
            "cassette-replay",
            "model-fixture",
        ]
        .into_iter()
        .map(CapabilityName::new)
        .collect()
    }

    /// The capabilities this profile explicitly DENIES: live network, live
    /// hardware, process spawn, wall-clock time, filesystem writes, and unseeded
    /// entropy. These are the Category D boundary; a recipe requesting one fails
    /// closed and its purpose becomes the denial.
    pub fn denied() -> Vec<CapabilityName> {
        vec![
            CapabilityName::new("net/http"),
            CapabilityName::new("net-connect"),
            CapabilityName::new("device-open"),
            CapabilityName::new("exec"),
            CapabilityName::new("process-spawn"),
            CapabilityName::new("clock-now"),
            CapabilityName::new("fs/write"),
            CapabilityName::new("fs-write"),
            CapabilityName::new("rng-unseeded"),
        ]
    }

    /// Whether this profile grants `capability`.
    pub fn grants(capability: &CapabilityName) -> bool {
        Self::granted().contains(capability)
    }

    /// Whether this profile explicitly denies `capability`.
    pub fn denies(capability: &CapabilityName) -> bool {
        Self::denied().contains(capability)
    }

    /// Seat `cx` with the profile: grant every granted capability through the
    /// host `seat` (minted with the `Cx` by [`sim_kernel::Cx::new_seated`]).
    ///
    /// Denied capabilities are never granted, so an op that demands one fails
    /// closed. Granting is idempotent, so re-seating a `Cx` is a no-op beyond the
    /// first call.
    pub fn seat(seat: &GrantSeat, cx: &mut Cx) -> Result<()> {
        for capability in Self::granted() {
            grant_into_result!(seat.grant(cx, capability))?;
        }
        Ok(())
    }
}

trait GrantOutcome {
    fn into_result(self) -> Result<()>;
}

impl GrantOutcome for () {
    fn into_result(self) -> Result<()> {
        Ok(())
    }
}

impl GrantOutcome for Result<()> {
    fn into_result(self) -> Result<()> {
        self
    }
}
