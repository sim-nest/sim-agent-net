//! Seed recipe registration for the crate-shipped cookbook books.

use sim_cookbook::{EmbeddedDir, RecipeStore, recipes_from_embedded};
use sim_kernel::{Cx, Error, Result};

/// Seed books embedded by crates that currently ship first-pass recipes.
pub const SEEDED_RECIPE_BOOKS: &[(&str, EmbeddedDir)] = &[
    ("core", sim_lib_core::RECIPES),
    ("codec/lisp", sim_codec_lisp::RECIPES),
    ("codec/json", sim_codec_json::RECIPES),
    ("numbers/f64", sim_lib_numbers_f64::RECIPES),
    ("numbers/complex", sim_lib_numbers_complex::RECIPES),
    ("numbers/func", sim_lib_numbers_func::RECIPES),
    ("numbers/cas", sim_lib_numbers_cas::RECIPES),
    ("numbers/rational", sim_lib_numbers_rational::RECIPES),
    ("numbers/i64", sim_lib_numbers_i64::RECIPES),
    ("numbers/arith", sim_lib_numbers_arith::RECIPES),
    ("numbers/bigint", sim_lib_numbers_bigint::RECIPES),
    ("numbers/bool", sim_lib_numbers_bool::RECIPES),
    ("numbers/tensor", sim_lib_numbers_tensor::RECIPES),
    (
        "numbers/tensor-cmplxf",
        sim_lib_numbers_tensor_cmplxf::RECIPES,
    ),
    (
        "numbers/tensor-rat64",
        sim_lib_numbers_tensor_rat64::RECIPES,
    ),
    // The newer codec embeds stay unseeded until their crates.io releases expose
    // RECIPES; codec/compare is a non-publishable developer harness. codec/bitwise
    // and codec/doc also require their own codec lib loaded to run.
    // COOK8.03 organs: each of these books' 01-basics recipe is a runnable compute
    // (`let`/`seq/map`/`match`), loaded on demand via its `requires`.
    ("organ/binding", sim_lib_binding::RECIPES),
    ("organ/sequence", sim_lib_sequence::RECIPES),
    ("organ/pattern", sim_lib_pattern::RECIPES),
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
    // discrete: seed every recipe EXCEPT matrix-runtime -- it reads a `#(discrete/Matrix)`
    // citizen class no runtime lib registers, so it cannot run in the cookbook sandbox and
    // must stay a source-only descriptor (COOKBOOK_7).
    register_book_except(
        &mut store,
        "discrete",
        sim_lib_discrete::RECIPES,
        &["matrix-runtime"],
    )?;
    // control: seed the runnable `if` recipe but skip the rich 30-agents descriptor
    // (`a30-021-physical-sensing`), which carries the extended agents schema and is
    // not runnable under the execute-everything verifier (kept source-only until
    // descriptors are first-class, COOK8.08).
    register_book_except(
        &mut store,
        "control",
        sim_lib_control::RECIPES,
        &["a30-021-physical-sensing"],
    )?;
    Ok(store)
}

/// Register every recipe in `dir` under `book`, skipping any whose id ends with an entry in
/// `skip` (a non-runnable descriptor kept source-only). `register_book` is all-or-nothing, so
/// a per-recipe skip goes through `recipes_from_embedded` + `insert_card`.
fn register_book_except(
    store: &mut RecipeStore,
    book: &str,
    dir: EmbeddedDir,
    skip: &[&str],
) -> Result<()> {
    let cards = recipes_from_embedded(dir)
        .map_err(|err| Error::Eval(format!("seed recipe book {book}: {err}")))?;
    for card in cards {
        if skip.iter().any(|s| card.id.ends_with(s)) {
            continue;
        }
        store
            .insert_card(card)
            .map_err(|err| Error::Eval(format!("seed recipe {book}: {err}")))?;
    }
    Ok(())
}

/// Install the cookbook runtime over the embedded seed recipe store.
pub fn install_seeded_cookbook_lib(cx: &mut Cx) -> Result<()> {
    crate::runtime::install_cookbook_lib(cx, seeded_recipe_store()?)
}
