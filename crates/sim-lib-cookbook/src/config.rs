//! Config-shaped provider for a host-owned cookbook loadable-lib directory.

use std::collections::HashSet;

use sim_config::{ConfigView, EffectiveConfig};
use sim_cookbook::EmbeddedDir;
use sim_kernel::{Expr, Symbol};
use sim_value::access::field_any;

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

    /// Returns the in-memory cookbook config snapshot.
    pub fn config(&self) -> &CookbookConfig {
        &self.config
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

/// Cookbook config provider backed by a merged effective config Dir.
pub struct ConfigCookbookProvider<'a, R: LoadableLibResolver + ?Sized> {
    inner: ConfigProvider<'a, R>,
}

impl<'a, R: LoadableLibResolver + ?Sized> ConfigCookbookProvider<'a, R> {
    /// Reads the effective `sim/cookbook` table and prepares a directory
    /// provider over `resolver`.
    pub fn new(effective: &EffectiveConfig, resolver: &'a R) -> Self {
        Self {
            inner: ConfigProvider::new(cookbook_config_from_effective(effective), resolver),
        }
    }

    /// Returns the effective cookbook config snapshot.
    pub fn config(&self) -> &CookbookConfig {
        self.inner.config()
    }

    /// Returns the configured minimum boot set without loading it.
    pub fn minimum_loaded(&self) -> &[String] {
        self.inner.minimum_loaded()
    }

    /// Resolves the configured loadable-lib directory.
    pub fn loadable_libs(&self) -> (LoadableLibList, Vec<String>) {
        self.inner.loadable_libs()
    }
}

/// Returns the stable config library id for cookbook defaults.
pub fn cookbook_lib_symbol() -> Symbol {
    Symbol::qualified("sim", "cookbook")
}

/// Builds the in-memory cookbook directory config from an effective Dir.
///
/// When no `sim/cookbook` table is present, the seeded built-in directory is
/// used. Once a table is present, its rows are authoritative, so a user config
/// can hide, subset, or reorder loadable libs without loading them.
pub fn cookbook_config_from_effective(effective: &EffectiveConfig) -> CookbookConfig {
    let Some(table) = effective.dir.table(&cookbook_lib_symbol()) else {
        return built_in_config();
    };
    let view = ConfigView::new(table);
    CookbookConfig {
        minimum_loaded: string_array(&view, "minimum_loaded"),
        loadable_libs: loadable_lib_rows(&view),
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

fn string_array(view: &ConfigView<'_>, key: &str) -> Vec<String> {
    view.list(key)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item {
            Expr::String(value) => Some(value.clone()),
            _ => None,
        })
        .collect()
}

fn loadable_lib_rows(view: &ConfigView<'_>) -> Vec<LoadableLibConfig> {
    view.list("loadable_lib")
        .unwrap_or_default()
        .iter()
        .map(|entry| LoadableLibConfig {
            id: string_at(entry, "id").unwrap_or_default().to_owned(),
            source: string_at(entry, "source").unwrap_or_default().to_owned(),
        })
        .collect()
}

fn string_at<'a>(entry: &'a Expr, key: &str) -> Option<&'a str> {
    field_any(entry, key).and_then(|value| match value {
        Expr::String(text) => Some(text.as_str()),
        _ => None,
    })
}
