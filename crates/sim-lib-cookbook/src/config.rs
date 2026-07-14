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

/// Overlay parsed from the `sim/cookbook` effective config table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookbookOverrides {
    /// Replacement host boot set, when the config explicitly names it.
    pub minimum_loaded: Option<Vec<String>>,
    /// Rows to add to, or replace in, the base loadable-lib directory.
    pub add: Vec<LoadableLibConfig>,
    /// Row ids hidden from the base loadable-lib directory.
    pub hide: Vec<String>,
    /// Preferred display order for row ids. Unmentioned rows keep relative order.
    pub order: Vec<String>,
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
        Self::new_with_base(effective, built_in_config(), resolver)
    }

    /// Reads the effective `sim/cookbook` table as an overlay over `base`.
    pub fn new_with_base(
        effective: &EffectiveConfig,
        base: CookbookConfig,
        resolver: &'a R,
    ) -> Self {
        Self {
            inner: ConfigProvider::new(
                cookbook_config_from_effective_with_base(effective, base),
                resolver,
            ),
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
/// used. When a table is present, it overlays the base directory so a host or
/// user config can add, hide, or reorder loadable libs without loading them.
pub fn cookbook_config_from_effective(effective: &EffectiveConfig) -> CookbookConfig {
    cookbook_config_from_effective_with_base(effective, built_in_config())
}

/// Builds the in-memory cookbook directory by applying effective overrides over
/// a host-provided base directory.
pub fn cookbook_config_from_effective_with_base(
    effective: &EffectiveConfig,
    base: CookbookConfig,
) -> CookbookConfig {
    apply_overrides(base, &cookbook_overrides_from_effective(effective))
}

/// Parses the effective `sim/cookbook` table as a directory overlay.
pub fn cookbook_overrides_from_effective(effective: &EffectiveConfig) -> CookbookOverrides {
    let Some(table) = effective.dir.table(&cookbook_lib_symbol()) else {
        return CookbookOverrides::default();
    };
    let view = ConfigView::new(table);
    CookbookOverrides {
        minimum_loaded: optional_string_array(&view, "minimum_loaded"),
        add: loadable_lib_rows(&view),
        hide: string_array(&view, "hide"),
        order: string_array(&view, "order"),
    }
}

/// Applies a cookbook config overlay over an existing directory snapshot.
pub fn apply_overrides(base: CookbookConfig, overrides: &CookbookOverrides) -> CookbookConfig {
    let CookbookConfig {
        minimum_loaded,
        loadable_libs,
    } = base;
    let minimum_loaded = overrides.minimum_loaded.clone().unwrap_or(minimum_loaded);
    let mut rows: Vec<LoadableLibConfig> = loadable_libs
        .into_iter()
        .filter(|row| !overrides.hide.contains(&row.id))
        .collect();
    for add in &overrides.add {
        match rows.iter_mut().find(|row| row.id == add.id) {
            Some(row) => *row = add.clone(),
            None => rows.push(add.clone()),
        }
    }
    if !overrides.order.is_empty() {
        let mut indexed = rows.into_iter().enumerate().collect::<Vec<_>>();
        indexed.sort_by_key(|(index, row)| {
            (
                overrides
                    .order
                    .iter()
                    .position(|id| id == &row.id)
                    .unwrap_or(usize::MAX),
                *index,
            )
        });
        rows = indexed.into_iter().map(|(_, row)| row).collect();
    }
    CookbookConfig {
        minimum_loaded,
        loadable_libs: rows,
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
        .filter_map(string_value)
        .collect()
}

fn optional_string_array(view: &ConfigView<'_>, key: &str) -> Option<Vec<String>> {
    view.get(key).map(|value| match value {
        Expr::List(items) => items.iter().filter_map(string_value).collect(),
        _ => Vec::new(),
    })
}

fn string_value(item: &Expr) -> Option<String> {
    match item {
        Expr::String(value) => Some(value.clone()),
        _ => None,
    }
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
