//! The `cookbook:` operations as registered callables.
//!
//! Each op is a [`CookbookOp`] closing over the shared [`RecipeStore`]. Listing
//! and showing ops extract the data they need under the store lock, drop it,
//! then build a runtime value (so the lock is never held across `Cx` work or a
//! recipe run, avoiding re-entrancy deadlocks). `cookbook:run` is capability
//! gated.

use std::sync::{Arc, Mutex};

use sim_cookbook::{RecipeCard, RecipeStore, next, ordered_cards, search, view};
use sim_kernel::{
    Args, Callable, ClassRef, Cx, Error, Expr, Object, ObjectCompat, Result, Symbol, Value,
};

use crate::build::{count_value, ids_value, recipe_value, run_value};
use crate::run::{decode_setup, require_eval_capability, run_recipe};

/// Which cookbook operation a [`CookbookOp`] performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    /// List the available cookbooks (`cookbook:books`).
    Books,
    /// List the chapters of a cookbook (`cookbook:chapters`).
    Chapters,
    /// List recipes (`cookbook:list`).
    List,
    /// Show one recipe (`cookbook:show`).
    Show,
    /// Show a recipe's setup steps (`cookbook:setup`).
    Setup,
    /// Search recipes (`cookbook:search`).
    Search,
    /// Suggest the next recipe (`cookbook:next`).
    Next,
    /// Reload the recipe store (`cookbook:reload`).
    Reload,
    /// Run a recipe (`cookbook:run`); capability gated.
    Run,
}

impl OpKind {
    /// Every op kind, for registration.
    pub const ALL: [OpKind; 9] = [
        OpKind::Books,
        OpKind::Chapters,
        OpKind::List,
        OpKind::Show,
        OpKind::Setup,
        OpKind::Search,
        OpKind::Next,
        OpKind::Reload,
        OpKind::Run,
    ];

    fn name(self) -> &'static str {
        match self {
            OpKind::Books => "books",
            OpKind::Chapters => "chapters",
            OpKind::List => "list",
            OpKind::Show => "show",
            OpKind::Setup => "setup",
            OpKind::Search => "search",
            OpKind::Next => "next",
            OpKind::Reload => "reload",
            OpKind::Run => "run",
        }
    }

    /// The `cookbook:<name>` symbol this op registers under.
    pub fn symbol(self) -> Symbol {
        Symbol::qualified("cookbook", self.name())
    }
}

/// A registered cookbook operation over a shared recipe store.
pub struct CookbookOp {
    store: Arc<Mutex<RecipeStore>>,
    kind: OpKind,
}

impl CookbookOp {
    /// Build the op for `kind` over `store`.
    pub fn new(store: Arc<Mutex<RecipeStore>>, kind: OpKind) -> Self {
        Self { store, kind }
    }

    fn lookup_card(&self, id: &str) -> Option<RecipeCard> {
        self.store
            .lock()
            .expect("recipe store lock")
            .card(id)
            .cloned()
    }
}

impl Object for CookbookOp {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.kind.symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for CookbookOp {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.resolve_class(&Symbol::qualified("core", "Function"))
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CookbookOp {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let args = args.into_vec();
        match self.kind {
            OpKind::Books => {
                let ids = self.with_store(|store| {
                    view(store)
                        .books
                        .iter()
                        .map(|b| b.id.clone())
                        .collect::<Vec<String>>()
                });
                ids_value(cx, &ids)
            }
            OpKind::Chapters => {
                let book = arg_string(cx, &args, "cookbook:chapters")?;
                let names = self.with_store(|store| {
                    view(store)
                        .books
                        .into_iter()
                        .find(|b| b.id == book)
                        .map(|b| {
                            b.chapters
                                .into_iter()
                                .map(|c| c.name)
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default()
                });
                ids_value(cx, &names)
            }
            OpKind::List => {
                let ids = self.with_store(|store| {
                    ordered_cards(store)
                        .iter()
                        .map(|c| c.id.clone())
                        .collect::<Vec<String>>()
                });
                ids_value(cx, &ids)
            }
            OpKind::Show => {
                let id = arg_string(cx, &args, "cookbook:show")?;
                let card = self.lookup_card(&id).ok_or_else(|| unknown(&id))?;
                recipe_value(cx, &card)
            }
            OpKind::Setup => {
                let id = arg_string(cx, &args, "cookbook:setup")?;
                let card = self.lookup_card(&id).ok_or_else(|| unknown(&id))?;
                let expr = decode_setup(cx, &card)?;
                cx.factory().expr(expr)
            }
            OpKind::Search => {
                let query = arg_string(cx, &args, "cookbook:search")?;
                let ids = self.with_store(|store| {
                    search(store, &query)
                        .iter()
                        .map(|c| c.id.clone())
                        .collect::<Vec<String>>()
                });
                ids_value(cx, &ids)
            }
            OpKind::Next => {
                let id = arg_string(cx, &args, "cookbook:next")?;
                let next_id = self.with_store(|store| next(store, &id).map(|c| c.id.clone()));
                match next_id {
                    Some(next_id) => cx.factory().string(next_id),
                    None => cx.factory().nil(),
                }
            }
            OpKind::Reload => {
                let count = self.store.lock().expect("recipe store lock").len();
                count_value(cx, count)
            }
            OpKind::Run => {
                let id = arg_string(cx, &args, "cookbook:run")?;
                let card = self.lookup_card(&id).ok_or_else(|| unknown(&id))?;
                require_eval_capability(cx)?;
                let run = run_recipe(cx, &card)?;
                run_value(cx, &run)
            }
        }
    }
}

impl CookbookOp {
    fn with_store<T>(&self, f: impl FnOnce(&RecipeStore) -> T) -> T {
        let store = self.store.lock().expect("recipe store lock");
        f(&store)
    }
}

fn unknown(id: &str) -> Error {
    Error::Eval(format!("unknown recipe `{id}`"))
}

fn arg_string(cx: &mut Cx, args: &[Value], op: &str) -> Result<String> {
    let value = args
        .first()
        .ok_or_else(|| Error::Eval(format!("{op} expects one argument")))?;
    match value.object().as_expr(cx)? {
        Expr::String(s) => Ok(s),
        Expr::Symbol(sym) => Ok(sym.to_string()),
        other => Err(Error::Eval(format!(
            "{op} expects a string id, got {other:?}"
        ))),
    }
}
