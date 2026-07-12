use std::sync::Arc;

use sim_cookbook::EmbeddedDir;
use sim_kernel::{
    AbiVersion, LibManifest, LibTarget, Linker, LoadCx, Result, Symbol, Version, library::Lib,
};
use sim_test_support::core_cx;

use crate::{
    ConfigProvider, CookbookConfig, LibCatalog, LoadableLibConfig, LoadableLibList,
    LoadableLibResolver, ResolvedLoadable,
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

#[cfg(feature = "seed-recipes")]
fn ids(directory: &LoadableLibList) -> Vec<&str> {
    directory
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect()
}
