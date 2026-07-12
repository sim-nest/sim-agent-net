//! Dynamic projection from a host loadable-lib directory to recipe cards.

use std::sync::Arc;

use sim_cookbook::{EmbeddedDir, RecipeCard, RecipeSource, RecipeStore};
use sim_kernel::{Cx, Error, Lib, Result};

use crate::catalog::LibCatalog;

/// Host-owned factory for constructing a loadable library.
pub type LibFactory = Arc<dyn Fn() -> Box<dyn Lib> + Send + Sync>;

/// One known library in the cookbook's effective loadable-lib directory.
pub struct LoadableLibEntry {
    /// Cookbook-facing library id, such as `numbers/cas`.
    pub id: String,
    /// Host resolver source key, such as `symbol:numbers/cas`.
    pub source: String,
    /// Human title used for this library's cookbook book.
    pub title: String,
    /// Book display order.
    pub order: i64,
    /// Embedded recipes for this lib, when the host can expose them.
    pub recipes: Option<EmbeddedDir>,
    /// Catalog instance used to resolve ordinary recipe `requires`.
    pub catalog_lib: Box<dyn Lib>,
    /// Factory used by lifecycle execution to create a fresh lib instance.
    pub factory: LibFactory,
}

/// Effective directory of host-loadable libraries known to the cookbook.
pub struct LoadableLibList {
    entries: Vec<LoadableLibEntry>,
}

impl LoadableLibList {
    /// Creates a directory from ordered entries.
    pub fn new(entries: Vec<LoadableLibEntry>) -> Self {
        Self { entries }
    }

    /// Borrows the entries in display order.
    pub fn entries(&self) -> &[LoadableLibEntry] {
        &self.entries
    }

    /// Finds an entry by exact cookbook id.
    pub fn entry(&self, id: &str) -> Option<&LoadableLibEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Whether a matching library is already loaded in `cx`.
    pub fn is_loaded(cx: &Cx, id: &str) -> bool {
        cx.registry().libs().iter().any(|loaded| {
            loaded.manifest.id.as_qualified_str() == id || loaded.manifest.id.name.as_ref() == id
        })
    }
}

impl LibCatalog for LoadableLibList {
    fn resolve(&self, name: &str) -> Option<&dyn Lib> {
        self.entries
            .iter()
            .find(|entry| entry.id == name || entry.id.rsplit('/').next() == Some(name))
            .map(|entry| entry.catalog_lib.as_ref())
    }
}

/// Builds the cookbook store for the current load state and known directory.
///
/// A known unloaded lib contributes one synthetic load recipe. A known loaded
/// lib contributes its embedded recipes, when available, followed by one
/// synthetic unload recipe sorted last in that book.
pub fn projected_recipe_store(cx: &Cx, directory: &LoadableLibList) -> Result<RecipeStore> {
    let mut store = RecipeStore::new();
    for entry in directory.entries() {
        if LoadableLibList::is_loaded(cx, &entry.id) {
            if let Some(recipes) = entry.recipes {
                store
                    .register_book(recipes)
                    .map_err(|err| Error::Eval(format!("{} recipes: {err}", entry.id)))?;
            }
            store.insert_card(unload_card(entry)).map_err(Error::Eval)?;
        } else {
            store.insert_card(load_card(entry)).map_err(Error::Eval)?;
        }
    }
    Ok(store)
}

fn load_card(entry: &LoadableLibEntry) -> RecipeCard {
    lifecycle_card(
        format!("cookbook/load/{}", entry.id),
        "cookbook/loadable".to_owned(),
        entry,
        "load",
        format!("Load {}", entry.id),
        0,
        0,
    )
}

fn unload_card(entry: &LoadableLibEntry) -> RecipeCard {
    lifecycle_card(
        format!("{}/cookbook-lifecycle/unload", entry.id),
        entry.id.clone(),
        entry,
        "unload",
        format!("Unload {}", entry.id),
        i64::MAX,
        i64::MAX,
    )
}

fn lifecycle_card(
    id: String,
    book: String,
    entry: &LoadableLibEntry,
    action: &str,
    title: String,
    chapter_order: i64,
    order: i64,
) -> RecipeCard {
    RecipeCard {
        id,
        book,
        chapter: "cookbook-lifecycle".to_owned(),
        chapter_title: "Lifecycle".to_owned(),
        chapter_summary: String::new(),
        title,
        codec: "lisp".to_owned(),
        setup: format!("(cookbook/{action}-lib {:?})", entry.id).into_bytes(),
        purpose: format!("{action} the loadable lib `{}`.", entry.id),
        order,
        chapter_order,
        book_order: entry.order,
        book_title: entry.title.clone(),
        book_summary: String::new(),
        tags: vec![
            format!("cookbook-action:{action}"),
            format!("cookbook-lib:{}", entry.id),
            format!("cookbook-source:{}", entry.source),
        ],
        requires: Vec::new(),
        expect: Vec::new(),
        source: RecipeSource::Crate {
            lib: "sim/cookbook".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use sim_kernel::{
        AbiVersion, LibManifest, LibTarget, Linker, LoadCx, Symbol, Version, library::Lib,
    };
    use sim_test_support::core_cx;

    use super::*;

    static DEMO_RECIPES: EmbeddedDir = &[
        ("book.toml", b"book = \"demo/lib\"\ntitle = \"Demo Lib\"\n"),
        (
            "01-basics/demo/recipe.toml",
            b"id = \"demo\"\ntitle = \"Demo\"\ncodec = \"lisp\"\nsetup = \"setup.siml\"\npurpose = \"purpose.md\"\n",
        ),
        ("01-basics/demo/setup.siml", b"(quote demo)"),
        ("01-basics/demo/purpose.md", b"demo recipe"),
    ];

    struct FixtureLib {
        namespace: &'static str,
        name: &'static str,
    }

    impl Lib for FixtureLib {
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

    fn fixture_lib() -> Box<dyn Lib> {
        Box::new(FixtureLib {
            namespace: "demo",
            name: "lib",
        })
    }

    fn directory() -> LoadableLibList {
        LoadableLibList::new(vec![LoadableLibEntry {
            id: "demo/lib".to_owned(),
            source: "symbol:demo/lib".to_owned(),
            title: "Demo Lib".to_owned(),
            order: 7,
            recipes: Some(DEMO_RECIPES),
            catalog_lib: fixture_lib(),
            factory: Arc::new(fixture_lib),
        }])
    }

    #[test]
    fn resolves_requires_by_full_id_and_tail() {
        let directory = directory();

        assert!(directory.resolve("demo/lib").is_some());
        assert!(directory.resolve("lib").is_some());
        assert!(directory.resolve("missing").is_none());
    }

    #[test]
    fn projected_store_contains_load_card_for_unloaded_lib() {
        let cx = core_cx();
        let store = projected_recipe_store(&cx, &directory()).unwrap();

        let cards = store.cards();
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.id, "cookbook/load/demo/lib");
        assert_eq!(card.book, "cookbook/loadable");
        assert!(card.tags.contains(&"cookbook-action:load".to_owned()));
        assert_eq!(card.book_order, 7);
    }

    #[test]
    fn projected_store_contains_loaded_recipes_and_unload_card() {
        let mut cx = core_cx();
        let lib = fixture_lib();
        cx.load_lib(lib.as_ref()).unwrap();

        let store = projected_recipe_store(&cx, &directory()).unwrap();

        assert!(store.card("demo/lib/01-basics/demo").is_some());
        let unload = store.card("demo/lib/cookbook-lifecycle/unload").unwrap();
        assert_eq!(unload.order, i64::MAX);
        assert_eq!(unload.chapter_order, i64::MAX);
        assert!(unload.tags.contains(&"cookbook-action:unload".to_owned()));
    }
}
