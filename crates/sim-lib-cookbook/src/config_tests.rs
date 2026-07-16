use std::{path::PathBuf, sync::Arc};

use sim_config::{ConfigDir, ConfigLayer, ConfigSource, ConfigTable, ConfigView, merge_layers};
use sim_cookbook::{EmbeddedDir, ordered_cards};
use sim_kernel::{
    AbiVersion, Expr, LibManifest, LibTarget, Linker, LoadCx, Result, Symbol, Version, library::Lib,
};
use sim_test_support::core_cx;

use crate::{
    ConfigCookbookProvider, ConfigProvider, CookbookConfig, LibCatalog, LoadableLibConfig,
    LoadableLibList, LoadableLibResolver, ResolvedLoadable,
    cookbook_config_from_effective_with_base, cookbook_lib_symbol, projected_recipe_store,
};

struct NamedLib {
    namespace: &'static str,
    name: &'static str,
}

impl Lib for NamedLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified(self.namespace, self.name),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: Vec::new(),
        }
    }

    fn load(&self, _cx: &mut LoadCx, _linker: &mut Linker) -> Result<()> {
        Ok(())
    }
}

struct FixtureResolver;

impl LoadableLibResolver for FixtureResolver {
    fn resolve(&self, source: &str, _id: &str) -> Option<ResolvedLoadable> {
        match source {
            "symbol:demo/alpha" => Some(resolved("Alpha", None, || {
                Box::new(NamedLib {
                    namespace: "demo",
                    name: "alpha",
                })
            })),
            "symbol:demo/beta" => Some(resolved("Beta", None, || {
                Box::new(NamedLib {
                    namespace: "demo",
                    name: "beta",
                })
            })),
            _ => None,
        }
    }
}

fn resolved<F>(title: &str, recipes: Option<EmbeddedDir>, make: F) -> ResolvedLoadable
where
    F: Fn() -> Box<dyn Lib + Send + Sync> + Send + Sync + 'static,
{
    ResolvedLoadable {
        title: title.to_owned(),
        recipes,
        factory: Arc::new(make),
    }
}

fn row(id: &str) -> LoadableLibConfig {
    LoadableLibConfig {
        id: id.to_owned(),
        source: format!("symbol:{id}"),
    }
}

#[test]
fn config_can_subset_and_reorder() {
    let config = CookbookConfig {
        minimum_loaded: Vec::new(),
        loadable_libs: vec![row("demo/beta"), row("demo/alpha")],
    };
    let provider = ConfigProvider::new(config, &FixtureResolver);

    let (directory, diagnostics) = provider.loadable_libs();

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let ids: Vec<_> = directory
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, ["demo/beta", "demo/alpha"]);
    assert_eq!(directory.entry("demo/beta").unwrap().order, 0);
    assert_eq!(directory.entry("demo/alpha").unwrap().order, 1);
    assert!(directory.resolve("beta").is_some());
    assert!(directory.resolve("alpha").is_some());
}

#[test]
fn unknown_source_and_duplicate_ids_are_diagnostics() {
    let config = CookbookConfig {
        minimum_loaded: Vec::new(),
        loadable_libs: vec![
            row("demo/alpha"),
            LoadableLibConfig {
                id: "demo/missing".to_owned(),
                source: "symbol:demo/missing".to_owned(),
            },
            LoadableLibConfig {
                id: "demo/alpha".to_owned(),
                source: "symbol:demo/beta".to_owned(),
            },
        ],
    };
    let provider = ConfigProvider::new(config, &FixtureResolver);

    let (directory, diagnostics) = provider.loadable_libs();

    assert_eq!(directory.entries().len(), 1);
    assert_eq!(directory.entries()[0].id, "demo/alpha");
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            == "unknown loadable-lib source `symbol:demo/missing` for `demo/missing`"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic == "duplicate loadable-lib id `demo/alpha`"),
        "{diagnostics:?}"
    );
}

#[test]
fn minimum_loaded_does_not_force_load() {
    let config = CookbookConfig {
        minimum_loaded: vec!["demo/alpha".to_owned()],
        loadable_libs: vec![row("demo/alpha")],
    };
    let provider = ConfigProvider::new(config, &FixtureResolver);
    let cx = core_cx();

    assert_eq!(provider.minimum_loaded(), ["demo/alpha"]);
    assert!(!LoadableLibList::is_loaded(&cx, "demo/alpha"));
}

#[test]
fn effective_config_drives_loadable_directory_without_loading() {
    let base = CookbookConfig {
        minimum_loaded: vec!["codec/lisp".to_owned()],
        loadable_libs: vec![row("demo/alpha"), row("demo/hidden")],
    };
    let override_table = map(vec![
        ("minimum_loaded", list(vec![text("demo/alpha")])),
        ("hide", list(vec![text("demo/hidden")])),
        ("order", list(vec![text("demo/beta"), text("demo/alpha")])),
        (
            "loadable_lib",
            list(vec![map(vec![
                ("id", text("demo/beta")),
                ("source", text("symbol:demo/beta")),
            ])]),
        ),
    ]);
    assert_provider_override_table_shape(&override_table);
    let effective = merge_layers(&[ConfigLayer::new(
        ConfigSource::Explicit {
            label: "work".to_owned(),
        },
        ConfigDir::one(cookbook_lib_symbol(), override_table).unwrap(),
    )]);
    let provider = ConfigCookbookProvider::new_with_base(&effective, base, &FixtureResolver);
    let cx = core_cx();

    let (directory, diagnostics) = provider.loadable_libs();

    assert_eq!(provider.minimum_loaded(), ["demo/alpha"]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(ids(&directory), ["demo/beta", "demo/alpha"]);
    assert!(directory.entry("demo/hidden").is_none());
    assert!(!LoadableLibList::is_loaded(&cx, "demo/alpha"));
    assert!(!LoadableLibList::is_loaded(&cx, "demo/beta"));
}

#[test]
fn home_work_config_can_hide_subset_and_reorder_cookbook_libs() {
    let base = CookbookConfig {
        minimum_loaded: vec!["codec/lisp".to_owned()],
        loadable_libs: vec![row("demo/alpha"), row("demo/hidden")],
    };
    let home = ConfigLayer::new(
        ConfigSource::HomeFile {
            path: PathBuf::from("/tmp/home/libs/sim/cookbook.toml"),
        },
        ConfigDir::one(
            cookbook_lib_symbol(),
            map(vec![("minimum_loaded", list(vec![text("codec/lisp")]))]),
        )
        .unwrap(),
    );
    let work = ConfigLayer::new(
        ConfigSource::WorkFile {
            path: PathBuf::from("/tmp/work/libs/sim/cookbook.toml"),
        },
        ConfigDir::one(
            cookbook_lib_symbol(),
            map(vec![
                ("hide", list(vec![text("demo/hidden")])),
                ("order", list(vec![text("demo/beta"), text("demo/alpha")])),
                (
                    "loadable_lib",
                    list(vec![map(vec![
                        ("id", text("demo/beta")),
                        ("source", text("symbol:demo/beta")),
                    ])]),
                ),
            ]),
        )
        .unwrap(),
    );
    let effective = merge_layers(&[home, work]);

    let config = cookbook_config_from_effective_with_base(&effective, base);
    let provider = ConfigProvider::new(config, &FixtureResolver);
    let cx = core_cx();
    let (directory, diagnostics) = provider.loadable_libs();

    assert_eq!(provider.minimum_loaded(), ["codec/lisp"]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(ids(&directory), ["demo/beta", "demo/alpha"]);
    assert!(directory.entry("demo/hidden").is_none());
    assert!(!LoadableLibList::is_loaded(&cx, "demo/alpha"));
    assert!(!LoadableLibList::is_loaded(&cx, "demo/beta"));

    let store = projected_recipe_store(&cx, &directory).unwrap();
    assert!(store.card("cookbook/load/demo/hidden").is_none());
    let cards = ordered_cards(&store);
    assert_eq!(
        cards
            .iter()
            .filter(|card| card.id == "cookbook/load/demo/beta")
            .count(),
        1
    );
    let load_ids: Vec<_> = cards
        .into_iter()
        .filter_map(|card| card.id.strip_prefix("cookbook/load/"))
        .collect();
    assert_eq!(load_ids, ["demo/beta", "demo/alpha"]);
}

#[cfg(feature = "seed-recipes")]
#[test]
fn built_in_provider_produces_seeded_directory() {
    let resolver = crate::BuiltInLoadableResolver;
    let provider = ConfigProvider::new(crate::built_in_config(), &resolver);

    let (from_config, diagnostics) = provider.loadable_libs();
    let seeded = crate::SeededLibCatalog::loadable_libs();

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(provider.minimum_loaded(), ["codec/lisp"]);
    assert_eq!(ids(&from_config), ids(&seeded));
    assert!(from_config.entry("numbers/cas").is_some());
    assert!(from_config.entry("codec/algol").is_some());
}

fn ids(directory: &LoadableLibList) -> Vec<&str> {
    directory
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect()
}

fn text(value: &str) -> Expr {
    Expr::String(value.to_owned())
}

fn list(items: Vec<Expr>) -> Expr {
    Expr::List(items)
}

fn map(entries: Vec<(&str, Expr)>) -> Expr {
    Expr::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Expr::Symbol(Symbol::new(key)), value))
            .collect(),
    )
}

fn assert_provider_override_table_shape(table: &Expr) {
    let table = ConfigTable::new(cookbook_lib_symbol(), table.clone()).unwrap();
    let view = ConfigView::new(&table);
    assert_string_list_field(&view, "minimum_loaded", &["demo/alpha"]);
    assert_string_list_field(&view, "hide", &["demo/hidden"]);
    assert_string_list_field(&view, "order", &["demo/beta", "demo/alpha"]);

    let rows = match view.get("loadable_lib") {
        Some(Expr::List(rows)) => rows,
        other => panic!("loadable_lib should be a repeated table list: {other:?}"),
    };
    assert_eq!(rows.len(), 1);
    let row = ConfigTable::new(
        Symbol::qualified("sim", "cookbook/loadable-lib"),
        rows[0].clone(),
    )
    .unwrap();
    let row = ConfigView::new(&row);
    assert_eq!(row.required_string("id").unwrap(), "demo/beta");
    assert_eq!(row.required_string("source").unwrap(), "symbol:demo/beta");
}

fn assert_string_list_field(view: &ConfigView<'_>, key: &str, expected: &[&str]) {
    let actual = match view.get(key) {
        Some(Expr::List(items)) => items
            .iter()
            .map(|item| match item {
                Expr::String(value) => value.as_str(),
                other => panic!("{key} should contain only strings: {other:?}"),
            })
            .collect::<Vec<_>>(),
        other => panic!("{key} should be a string list: {other:?}"),
    };
    assert_eq!(actual, expected);
}
