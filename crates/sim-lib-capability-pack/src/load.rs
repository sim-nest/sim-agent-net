use crate::{LibrarySpec, ValidatedPack};
use sim_kernel::Symbol;

/// Injected host loader. It receives only a completely validated closure.
pub trait LibraryLoader {
    /// Loads one already-resolved library route.
    fn load(&mut self, library: &LibrarySpec) -> Result<(), String>;
}
/// Successfully loaded composition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPack {
    /// Root id.
    pub root: crate::ContentId,
    /// Dependency-first loaded libraries.
    pub libraries: Vec<Symbol>,
}
/// Host-load failure after successful preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    /// Library that failed.
    pub library: Symbol,
    /// Host diagnostic.
    pub detail: String,
}
/// Loads a validated pack without rebuilding it or changing the bootloader frame.
pub fn load(pack: ValidatedPack, loader: &mut dyn LibraryLoader) -> Result<LoadedPack, LoadError> {
    let mut loaded = vec![];
    for library in &pack.libraries {
        loader.load(library).map_err(|detail| LoadError {
            library: library.id.clone(),
            detail,
        })?;
        loaded.push(library.id.clone());
    }
    Ok(LoadedPack {
        root: pack.resolved.root,
        libraries: loaded,
    })
}
