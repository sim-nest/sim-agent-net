//! Content-addressed, checked composition of loadable SIM capabilities.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codec;
mod load;
mod model;
mod resolve;
mod validate;

pub use codec::{CodecError, decode_pack, encode_pack};
pub use load::{LibraryLoader, LoadError, LoadedPack, load};
pub use model::{
    CURRENT_PACK_VERSION, CapabilityPack, CheckSpecimen, ContentId, Import, LibrarySpec,
    ManualFallback, PackClaim, PackOutput, PackSurface,
};
pub use resolve::{PackDir, ResolveError, ResolvedPack, resolve};
pub use validate::{Catalog, ValidatedPack, ValidationError, validate};

/// Registers the public pack records as Lisp read-construct Citizens and Shapes.
pub fn register_citizens(registry: &mut sim_citizen::CitizenRegistry) -> sim_kernel::Result<()> {
    registry.register::<CapabilityPack>()?;
    Ok(())
}

#[cfg(test)]
mod tests;
