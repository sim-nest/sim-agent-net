use crate::{ContentId, LibrarySpec, ResolvedPack};
use sim_kernel::Symbol;
use std::collections::{BTreeMap, BTreeSet};

/// Read-only Index/Shape catalog used during preflight.
pub trait Catalog {
    /// Whether a route resolves.
    fn has_route(&self, route: &Symbol) -> bool;
    /// Whether a Shape resolves.
    fn has_shape(&self, shape: &Symbol) -> bool;
    /// Declared effects for a routed implementation.
    fn effects(&self, route: &Symbol) -> Option<BTreeSet<Symbol>>;
    /// Whether a disclosure class is known.
    fn has_disclosure(&self, disclosure: &Symbol) -> bool;
}

/// Fully preflighted closure. Construction is possible only through [`validate`].
#[derive(Clone, Debug)]
pub struct ValidatedPack {
    pub(crate) resolved: ResolvedPack,
    pub(crate) libraries: Vec<LibrarySpec>,
}

/// Preflight refusal; no library has loaded when returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// Malformed open record.
    Malformed(String),
    /// Capability exceeds effective ceiling.
    AuthorityWidening {
        /// Pack.
        pack: ContentId,
        /// Capability.
        capability: Symbol,
    },
    /// Missing route.
    MissingRoute(Symbol),
    /// Missing Shape.
    MissingShape(Symbol),
    /// Output conflict.
    ConflictingOutput(Symbol),
    /// Unknown surface disclosure.
    UnknownDisclosure(Symbol),
    /// Declared effects differ from catalog.
    EffectMismatch(Symbol),
    /// Claim names absent library.
    MissingLibrary(Symbol),
    /// Missing success/refusal specimen.
    MissingSpecimen(&'static str),
    /// No manual fallback.
    MissingFallback,
}

/// Resolves all routes, Shapes, effects, surfaces, outputs, claims, specimens, and fallbacks.
pub fn validate(
    resolved: ResolvedPack,
    catalog: &dyn Catalog,
) -> Result<ValidatedPack, ValidationError> {
    let mut libraries = Vec::new();
    let mut ids = BTreeSet::new();
    let mut outputs = BTreeMap::new();
    let mut outcomes = BTreeSet::new();
    let mut fallback = false;
    for (pack_id, pack) in &resolved.packs {
        let local = pack.typed_libraries().map_err(ValidationError::Malformed)?;
        for lib in &local {
            if !catalog.has_route(&lib.route) {
                return Err(ValidationError::MissingRoute(lib.route.clone()));
            }
            if !catalog.has_shape(&lib.shape) {
                return Err(ValidationError::MissingShape(lib.shape.clone()));
            }
            if catalog.effects(&lib.route).as_ref() != Some(&lib.effects) {
                return Err(ValidationError::EffectMismatch(lib.route.clone()));
            }
            ids.insert(lib.id.clone());
        }
        for claim in pack.typed_claims().map_err(ValidationError::Malformed)? {
            if !local.iter().any(|l| l.id == claim.library) {
                return Err(ValidationError::MissingLibrary(claim.library));
            }
            if !resolved.ceilings[pack_id].contains(&claim.capability) {
                return Err(ValidationError::AuthorityWidening {
                    pack: pack_id.clone(),
                    capability: claim.capability,
                });
            }
        }
        for output in pack.typed_outputs().map_err(ValidationError::Malformed)? {
            if !local.iter().any(|l| l.id == output.library) {
                return Err(ValidationError::MissingLibrary(output.library));
            }
            if outputs
                .insert(output.name.clone(), output.library)
                .is_some()
            {
                return Err(ValidationError::ConflictingOutput(output.name));
            }
        }
        for surface in pack.typed_surfaces().map_err(ValidationError::Malformed)? {
            if !catalog.has_disclosure(&surface.disclosure) {
                return Err(ValidationError::UnknownDisclosure(surface.disclosure));
            }
        }
        outcomes.extend(
            pack.typed_specimens()
                .map_err(ValidationError::Malformed)?
                .into_iter()
                .map(|s| s.outcome),
        );
        fallback |= pack
            .typed_fallbacks()
            .map_err(ValidationError::Malformed)?
            .iter()
            .any(|f| !f.instruction.trim().is_empty());
        libraries.extend(local);
    }
    for needed in ["success", "refusal"] {
        if !outcomes.contains(&Symbol::new(needed)) {
            return Err(ValidationError::MissingSpecimen(needed));
        }
    }
    if !fallback {
        return Err(ValidationError::MissingFallback);
    }
    Ok(ValidatedPack {
        resolved,
        libraries,
    })
}
