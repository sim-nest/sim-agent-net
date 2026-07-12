//! Config-shaped provider for a host-owned cookbook loadable-lib directory.

use std::collections::HashSet;

use sim_cookbook::EmbeddedDir;

use crate::loadable::{LibFactory, LoadableLibEntry, LoadableLibList};

/// In-memory shape of the `sim/cookbook` config table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookbookConfig {
    /// Host boot set that a caller may load separately.
    pub minimum_loaded: Vec<String>,
    /// Ordered effective directory of loadable libs to expose in the cookbook.
    pub loadable_libs: Vec<LoadableLibConfig>,
}

/// One configured loadable-lib row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadableLibConfig {
    /// Cookbook-facing library id, such as `numbers/cas`.
    pub id: String,
    /// Host resolver key, such as `symbol:numbers/cas`.
    pub source: String,
}

/// Host-resolved loadable-lib material.
pub struct ResolvedLoadable {
    /// Human title used for this library's cookbook book.
    pub title: String,
    /// Embedded recipes for this lib, when the host can expose them.
    pub recipes: Option<EmbeddedDir>,
    /// Factory used to build fresh lib instances.
    pub factory: LibFactory,
}

/// Host resolver for config-selected loadable libs.
pub trait LoadableLibResolver {
    /// Resolves one config row by source key and cookbook id.
    fn resolve(&self, source: &str, id: &str) -> Option<ResolvedLoadable>;
}

/// Converts a [`CookbookConfig`] into an effective loadable-lib directory.
pub struct ConfigProvider<'a, R: LoadableLibResolver + ?Sized> {
    config: CookbookConfig,
    resolver: &'a R,
}

impl<'a, R: LoadableLibResolver + ?Sized> ConfigProvider<'a, R> {
    /// Creates a provider over one config snapshot and host resolver.
    pub fn new(config: CookbookConfig, resolver: &'a R) -> Self {
        Self { config, resolver }
    }

    /// Returns the configured minimum boot set without loading it.
    pub fn minimum_loaded(&self) -> &[String] {
        &self.config.minimum_loaded
    }

    /// Resolves the configured loadable-lib directory.
    ///
    /// The config array order is the resulting display order. Unknown sources
    /// and duplicate ids are reported as diagnostics and skipped.
    pub fn loadable_libs(&self) -> (LoadableLibList, Vec<String>) {
        let mut entries = Vec::new();
        let mut diagnostics = Vec::new();
        let mut seen = HashSet::new();

        for (index, cfg) in self.config.loadable_libs.iter().enumerate() {
            if !seen.insert(cfg.id.as_str()) {
                diagnostics.push(format!("duplicate loadable-lib id `{}`", cfg.id));
                continue;
            }

            match self.resolver.resolve(&cfg.source, &cfg.id) {
                Some(resolved) => entries.push(LoadableLibEntry {
                    id: cfg.id.clone(),
                    source: cfg.source.clone(),
                    title: resolved.title,
                    order: index as i64,
                    recipes: resolved.recipes,
                    catalog_lib: (resolved.factory)(),
                    factory: resolved.factory,
                }),
                None => diagnostics.push(format!(
                    "unknown loadable-lib source `{}` for `{}`",
                    cfg.source, cfg.id
                )),
            }
        }

        (LoadableLibList::new(entries), diagnostics)
    }
}

/// Built-in cookbook directory config used by the seeded host resolver.
pub fn built_in_config() -> CookbookConfig {
    CookbookConfig {
        minimum_loaded: vec!["codec/lisp".to_owned()],
        loadable_libs: vec![
            loadable("numbers/i64"),
            loadable("numbers/arith"),
            loadable("numbers/bigint"),
            loadable("numbers/bool"),
            loadable("numbers/f64"),
            loadable("numbers/rational"),
            loadable("numbers/complex"),
            loadable("numbers/func"),
            loadable("numbers/cas"),
            loadable("numbers/tensor"),
            loadable("numbers/tensor-bcast"),
            loadable("discrete"),
            loadable("organ/binding"),
            loadable("organ/control"),
            loadable("organ/sequence"),
            loadable("organ/pattern"),
            loadable("codec/algol"),
            loadable("codec/scheme-r7rs-small"),
            loadable("midi/digest"),
        ],
    }
}

fn loadable(id: &str) -> LoadableLibConfig {
    LoadableLibConfig {
        id: id.to_owned(),
        source: format!("symbol:{id}"),
    }
}
