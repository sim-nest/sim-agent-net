//! Dynamic projection from a host loadable-lib directory to recipe cards.

use std::sync::Arc;

use sim_cookbook::{
    EmbeddedDir, RecipeCard, RecipeRun, RecipeSource, RecipeStore, recipes_from_embedded,
};
use sim_kernel::{Cx, Error, Lib, LibId, Result, Symbol};

use crate::catalog::LibCatalog;

/// Host-owned factory for constructing a loadable library.
pub type LibFactory = Arc<dyn Fn() -> Box<dyn Lib + Send + Sync> + Send + Sync>;

/// Lifecycle command encoded by a synthetic cookbook card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleAction {
    /// Load a known library from the host-owned directory.
    Load,
    /// Unload a currently loaded library with the kernel's non-cascade unload.
    Unload,
}

impl LifecycleAction {
    /// Stable action tag value used by lifecycle cards.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Unload => "unload",
        }
    }
}

/// One known library in the cookbook's effective loadable-lib directory.
pub struct LoadableLibEntry {
    /// Cookbook-facing library id, such as `numbers/cas`.
    pub id: String,
    /// Host resolver source key, such as `symbol:numbers/cas`.
    pub source: String,
    /// Human title for this library's cookbook book.
    pub title: String,
    /// Book display order.
    pub order: i64,
    /// Embedded recipes for this lib, when the host can expose them.
    pub recipes: Option<EmbeddedDir>,
    /// Catalog instance that resolves ordinary recipe `requires`.
    pub catalog_lib: Box<dyn Lib + Send + Sync>,
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
        Self::loaded_id(cx, id).is_some()
    }

    /// Load a known library from its host factory.
    ///
    /// Calling this for an already-loaded id is a successful no-op, so a stale
    /// load card cannot duplicate registry entries.
    pub fn load(&self, cx: &mut Cx, id: &str) -> Result<String> {
        let entry = self
            .entry(id)
            .ok_or_else(|| Error::Eval(format!("unknown loadable lib `{id}`")))?;
        if Self::loaded_entry_id(cx, entry).is_some() {
            return Ok(format!("already loaded {id}"));
        }
        let lib = (entry.factory)();
        cx.load_lib(lib.as_ref())?;
        Ok(format!("loaded {id}"))
    }

    /// Unload a loaded library with the kernel's bare, non-cascade unload.
    ///
    /// If another loaded lib depends on this one, the kernel refusal is returned
    /// unchanged to the lifecycle runner, which reports it as `ok:false`.
    pub fn unload(&self, cx: &mut Cx, id: &str) -> Result<String> {
        let loaded_id = self
            .entry(id)
            .and_then(|entry| Self::loaded_entry_id(cx, entry))
            .or_else(|| Self::loaded_id(cx, id))
            .ok_or_else(|| Error::Eval(format!("lib `{id}` is not loaded")))?;
        cx.unload_lib(loaded_id)?;
        Ok(format!("unloaded {id}"))
    }

    fn loaded_id(cx: &Cx, id: &str) -> Option<LibId> {
        cx.registry()
            .libs()
            .iter()
            .find(|loaded| lib_id_matches(&loaded.manifest.id, id))
            .map(|loaded| loaded.id)
    }

    fn loaded_entry_id(cx: &Cx, entry: &LoadableLibEntry) -> Option<LibId> {
        let manifest_id = entry.catalog_lib.manifest().id;
        cx.registry()
            .libs()
            .iter()
            .find(|loaded| {
                loaded.manifest.id == manifest_id
                    || lib_id_matches(&loaded.manifest.id, &entry.id)
                    || lib_id_matches(&loaded.manifest.id, &manifest_id.as_qualified_str())
            })
            .map(|loaded| loaded.id)
    }
}

impl LibCatalog for LoadableLibList {
    fn resolve(&self, name: &str) -> Option<&dyn Lib> {
        self.entries
            .iter()
            .find(|entry| {
                entry.id == name
                    || entry.id.rsplit('/').next() == Some(name)
                    || lib_id_matches(&entry.catalog_lib.manifest().id, name)
            })
            .map(|entry| entry.catalog_lib.as_ref() as &dyn Lib)
    }
}

fn lib_id_matches(symbol: &Symbol, id: &str) -> bool {
    let tail = id.rsplit('/').next().unwrap_or(id);
    let qualified = symbol.as_qualified_str();
    qualified == id
        || qualified.replace('/', "-") == id
        || symbol.name.as_ref() == id
        || symbol.name.as_ref() == tail
}

/// Reads the lifecycle action encoded by a synthetic cookbook card.
pub fn lifecycle_action(card: &RecipeCard) -> Option<(LifecycleAction, String)> {
    let action = card
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("cookbook-action:"))?;
    let lib = card
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("cookbook-lib:"))?;
    let action = match action {
        "load" => LifecycleAction::Load,
        "unload" => LifecycleAction::Unload,
        _ => return None,
    };
    Some((action, lib.to_owned()))
}

/// Run a cookbook card against a dynamic loadable-lib directory.
///
/// Lifecycle cards execute directly against the live `Cx` registry. Ordinary
/// recipe cards still use the existing requires-driven catalog runner, with the
/// same directory as their [`LibCatalog`] resolver.
pub fn run_recipe_with_loadable_libs(
    cx: &mut Cx,
    directory: &LoadableLibList,
    card: &RecipeCard,
) -> Result<RecipeRun> {
    if let Some((action, lib)) = lifecycle_action(card) {
        return Ok(run_lifecycle_action(cx, directory, action, &lib, &card.id));
    }
    crate::run::run_recipe_with_catalog(cx, directory, card)
}

/// Execute one lifecycle command and package it as a [`RecipeRun`].
pub fn run_lifecycle_action(
    cx: &mut Cx,
    directory: &LoadableLibList,
    action: LifecycleAction,
    lib: &str,
    recipe: &str,
) -> RecipeRun {
    let result = match action {
        LifecycleAction::Load => directory.load(cx, lib),
        LifecycleAction::Unload => directory.unload(cx, lib),
    };
    lifecycle_run(recipe, result)
}

fn lifecycle_run(recipe: &str, result: Result<String>) -> RecipeRun {
    match result {
        Ok(message) => RecipeRun {
            recipe: recipe.to_owned(),
            forms: 1,
            results: vec![message],
            checks: Vec::new(),
            ok: true,
        },
        Err(err) => RecipeRun {
            recipe: recipe.to_owned(),
            forms: 1,
            results: vec![err.to_string()],
            checks: Vec::new(),
            ok: false,
        },
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
        if LoadableLibList::loaded_entry_id(cx, entry).is_some() {
            if let Some(recipes) = entry.recipes {
                for mut card in recipes_from_embedded(recipes)
                    .map_err(|err| Error::Eval(format!("{} recipes: {err}", entry.id)))?
                {
                    card.id = effective_recipe_id(&card.book, &card.id, &entry.id);
                    card.book = entry.id.clone();
                    card.book_title = entry.title.clone();
                    card.book_order = entry.order;
                    store.insert_card(card).map_err(Error::Eval)?;
                }
            } else {
                store
                    .insert_card(setup_debt_card(entry))
                    .map_err(Error::Eval)?;
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
        LifecycleAction::Load,
        format!("Load {}", entry.id),
        0,
        0,
    )
}

fn effective_recipe_id(original_book: &str, original_id: &str, entry_id: &str) -> String {
    if original_id == original_book {
        return entry_id.to_owned();
    }
    if let Some(suffix) = original_id.strip_prefix(&format!("{original_book}/")) {
        return format!("{entry_id}/{suffix}");
    }
    if original_id == entry_id || original_id.starts_with(&format!("{entry_id}/")) {
        return original_id.to_owned();
    }
    format!("{entry_id}/{original_id}")
}

fn unload_card(entry: &LoadableLibEntry) -> RecipeCard {
    lifecycle_card(
        format!("{}/cookbook-lifecycle/unload", entry.id),
        entry.id.clone(),
        entry,
        LifecycleAction::Unload,
        format!("Unload {}", entry.id),
        i64::MAX,
        i64::MAX,
    )
}

fn setup_debt_card(entry: &LoadableLibEntry) -> RecipeCard {
    RecipeCard {
        id: format!("{}/cookbook-lifecycle/setup-debt", entry.id),
        book: entry.id.clone(),
        chapter: "cookbook-lifecycle".to_owned(),
        chapter_title: "Lifecycle".to_owned(),
        chapter_summary: String::new(),
        title: format!("Setup debt for {}", entry.id),
        codec: "lisp".to_owned(),
        setup: format!(
            "(cookbook/setup-debt {:?} {:?})",
            "missing-recipes", entry.id
        )
        .into_bytes(),
        purpose: format!(
            "`{}` is loadable in this product build and exposes no embedded cookbook directory; this descriptor keeps the gap visible.",
            entry.id
        ),
        order: i64::MAX - 1,
        chapter_order: i64::MAX - 1,
        book_order: entry.order,
        book_title: entry.title.clone(),
        book_summary: String::new(),
        tags: vec![
            "sandbox-descriptor".to_owned(),
            "setup-debt:missing-recipes".to_owned(),
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

fn lifecycle_card(
    id: String,
    book: String,
    entry: &LoadableLibEntry,
    action: LifecycleAction,
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
        setup: format!("(cookbook/{}-lib {:?})", action.as_str(), entry.id).into_bytes(),
        purpose: format!("{} the loadable lib `{}`.", action.as_str(), entry.id),
        order,
        chapter_order,
        book_order: entry.order,
        book_title: entry.title.clone(),
        book_summary: String::new(),
        tags: vec![
            format!("cookbook-action:{}", action.as_str()),
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
