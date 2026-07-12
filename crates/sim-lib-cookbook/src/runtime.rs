//! The cookbook lib: registers the `cookbook:` ops over a shared recipe store.
//!
//! A server or CLI builds a [`RecipeStore`] (by registering each loaded lib's
//! embedded book and loading the user overlay), then installs this lib to
//! expose the recipes as runtime operations.

use std::sync::{Arc, Mutex};

use sim_cookbook::RecipeStore;
use sim_kernel::{
    AbiVersion, CORE_TABLE_CLASS_ID, ClassRef, Cx, Export, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};

use crate::cli::{cookbook_cli_exports, register_cookbook_cli};
use crate::loadable::{LifecycleAction, LoadableLibList};
use crate::ops::{CookbookLifecycleOp, CookbookOp, OpKind};

/// The lib id: `sim:cookbook`.
pub fn manifest_name() -> Symbol {
    Symbol::qualified("sim", "cookbook")
}

/// The registered value holding the shared cookbook store handle.
pub fn store_symbol() -> Symbol {
    Symbol::qualified("cookbook", "store")
}

/// A runtime value that exposes the shared recipe store to projection layers.
#[sim_citizen_derive::non_citizen(
    reason = "live cookbook store handle; reconstruct from embedded recipe descriptors",
    kind = "handle",
    descriptor = "cookbook/Recipe"
)]
#[derive(Clone)]
pub struct CookbookStoreHandle {
    store: Arc<Mutex<RecipeStore>>,
}

impl CookbookStoreHandle {
    /// Build a handle over the store used by the `cookbook:` ops.
    pub fn new(store: Arc<Mutex<RecipeStore>>) -> Self {
        Self { store }
    }

    /// Return the shared store handle.
    pub fn store(&self) -> Arc<Mutex<RecipeStore>> {
        self.store.clone()
    }
}

impl Object for CookbookStoreHandle {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<cookbook-store>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for CookbookStoreHandle {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(CORE_TABLE_CLASS_ID, Symbol::qualified("core", "Table"))
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let count = self.store.lock().expect("recipe store lock").len();
        let kind = cx
            .factory()
            .symbol(Symbol::qualified("cookbook", "store"))?;
        let recipes = cx.factory().string(count.to_string())?;
        cx.factory().table(vec![
            (Symbol::new("kind"), kind),
            (Symbol::new("recipes"), recipes),
        ])
    }
}

/// The cookbook lib, holding the shared recipe store the ops read.
pub struct CookbookLib {
    store: Arc<Mutex<RecipeStore>>,
    loadable_libs: Arc<LoadableLibList>,
}

impl CookbookLib {
    /// Build the lib over an already-populated recipe store.
    pub fn new(store: RecipeStore) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            loadable_libs: Arc::new(LoadableLibList::new(Vec::new())),
        }
    }

    /// Build the lib over `store` with a host-owned loadable-lib directory.
    pub fn with_loadable_libs(store: RecipeStore, loadable_libs: LoadableLibList) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            loadable_libs: Arc::new(loadable_libs),
        }
    }

    /// The shared store handle (for the server to keep populating, if needed).
    pub fn store(&self) -> Arc<Mutex<RecipeStore>> {
        self.store.clone()
    }
}

impl Lib for CookbookLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: manifest_name(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: {
                let mut exports = op_exports();
                exports.extend(cookbook_cli_exports());
                exports
            },
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for kind in OpKind::ALL {
            let op = CookbookOp::new(self.store.clone(), kind);
            linker.function_value(kind.symbol(), cx.factory().opaque(Arc::new(op))?)?;
        }
        for action in [LifecycleAction::Load, LifecycleAction::Unload] {
            let op = CookbookLifecycleOp::new(self.loadable_libs.clone(), action);
            linker.function_value(
                CookbookLifecycleOp::symbol(action),
                cx.factory().opaque(Arc::new(op))?,
            )?;
        }
        let store = CookbookStoreHandle::new(self.store.clone());
        linker.value(store_symbol(), cx.factory().opaque(Arc::new(store))?)?;
        register_cookbook_cli(cx, linker)?;
        Ok(())
    }
}

/// The `cookbook:*` function exports.
pub fn op_exports() -> Vec<Export> {
    let mut exports = OpKind::ALL
        .into_iter()
        .map(|kind| Export::Function {
            symbol: kind.symbol(),
            function_id: None,
        })
        .collect::<Vec<_>>();
    exports.extend(
        [LifecycleAction::Load, LifecycleAction::Unload]
            .into_iter()
            .map(|action| Export::Function {
                symbol: CookbookLifecycleOp::symbol(action),
                function_id: None,
            }),
    );
    exports.push(Export::Value {
        symbol: store_symbol(),
    });
    exports
}

/// Install the cookbook lib over `store` (idempotent).
pub fn install_cookbook_lib(cx: &mut Cx, store: RecipeStore) -> Result<()> {
    sim_lib_core::install_once(cx, &CookbookLib::new(store))?;
    Ok(())
}

/// Install the cookbook lib with lifecycle ops over `loadable_libs`.
pub fn install_cookbook_lib_with_loadable_libs(
    cx: &mut Cx,
    store: RecipeStore,
    loadable_libs: LoadableLibList,
) -> Result<()> {
    sim_lib_core::install_once(cx, &CookbookLib::with_loadable_libs(store, loadable_libs))?;
    Ok(())
}
