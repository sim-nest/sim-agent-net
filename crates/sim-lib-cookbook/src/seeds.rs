//! Seed recipe registration for the crate-shipped cookbook books.

use sim_cookbook::{EmbeddedDir, RecipeStore};
use sim_kernel::{Cx, Error, Result};

/// Seed books embedded by crates that currently ship first-pass recipes.
pub const SEEDED_RECIPE_BOOKS: &[(&str, EmbeddedDir)] = &[
    ("core", sim_lib_core::RECIPES),
    ("codec/lisp", sim_codec_lisp::RECIPES),
    ("codec/json", sim_codec_json::RECIPES),
    ("numbers/f64", sim_lib_numbers_f64::RECIPES),
    ("numbers/complex", sim_lib_numbers_complex::RECIPES),
    ("organ/binding", sim_lib_binding::RECIPES),
    ("organ/dispatch", sim_lib_dispatch::RECIPES),
    ("stream-core-shapes", sim_lib_stream_core::RECIPES),
    ("agent-runner-core", sim_lib_agent_runner_core::RECIPES),
];

/// Build a store populated from the embedded seed recipe books.
pub fn seeded_recipe_store() -> Result<RecipeStore> {
    let mut store = RecipeStore::new();
    for (book, recipes) in SEEDED_RECIPE_BOOKS {
        store
            .register_book(recipes)
            .map_err(|err| Error::Eval(format!("seed recipe book {book}: {err}")))?;
    }
    Ok(store)
}

/// Install the cookbook runtime over the embedded seed recipe store.
pub fn install_seeded_cookbook_lib(cx: &mut Cx) -> Result<()> {
    crate::runtime::install_cookbook_lib(cx, seeded_recipe_store()?)
}
