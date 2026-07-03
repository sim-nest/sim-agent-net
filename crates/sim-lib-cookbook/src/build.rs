//! Build runtime `Value`s (tables and lists of strings) from cookbook data, so
//! the `cookbook:` ops return objects the rest of the runtime can consume.
//!
//! Integers are rendered as strings to avoid coupling the cookbook to any
//! number domain; the structured shape (tables, lists, bools) is what matters
//! to the CLI and WebUI consumers.

use sim_cookbook::{RecipeCard, RecipeRun};
use sim_kernel::{Cx, Result, Symbol, Value};

fn key(name: &str) -> Symbol {
    Symbol::new(name)
}

fn string(cx: &mut Cx, value: &str) -> Result<Value> {
    cx.factory().string(value.to_string())
}

fn string_list(cx: &mut Cx, items: &[String]) -> Result<Value> {
    let values = items
        .iter()
        .map(|item| cx.factory().string(item.clone()))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(values)
}

/// A list of strings as a runtime list value.
pub fn ids_value(cx: &mut Cx, ids: &[String]) -> Result<Value> {
    string_list(cx, ids)
}

/// One recipe as a table value (id, book, chapter, title, codec, purpose,
/// tags, requires).
pub fn recipe_value(cx: &mut Cx, card: &RecipeCard) -> Result<Value> {
    let id = string(cx, &card.id)?;
    let book = string(cx, &card.book)?;
    let chapter = string(cx, &card.chapter)?;
    let title = string(cx, &card.title)?;
    let codec = string(cx, &card.codec)?;
    let purpose = string(cx, &card.purpose)?;
    let tags = string_list(cx, &card.tags)?;
    let requires = string_list(cx, &card.requires)?;
    cx.factory().table(vec![
        (key("id"), id),
        (key("book"), book),
        (key("chapter"), chapter),
        (key("title"), title),
        (key("codec"), codec),
        (key("purpose"), purpose),
        (key("tags"), tags),
        (key("requires"), requires),
    ])
}

/// A `RecipeRun` as a table value (recipe, ok, forms, results, checks).
pub fn run_value(cx: &mut Cx, run: &RecipeRun) -> Result<Value> {
    let recipe = string(cx, &run.recipe)?;
    let ok = cx.factory().bool(run.ok)?;
    let forms = string(cx, &run.forms.to_string())?;
    let results = string_list(cx, &run.results)?;
    let mut check_values = Vec::new();
    for check in &run.checks {
        let form = string(cx, &check.form.to_string())?;
        let expected = string(cx, &check.expected)?;
        let actual = string(cx, &check.actual)?;
        let pass = cx.factory().bool(check.pass)?;
        check_values.push(cx.factory().table(vec![
            (key("form"), form),
            (key("expected"), expected),
            (key("actual"), actual),
            (key("pass"), pass),
        ])?);
    }
    let checks = cx.factory().list(check_values)?;
    cx.factory().table(vec![
        (key("recipe"), recipe),
        (key("ok"), ok),
        (key("forms"), forms),
        (key("results"), results),
        (key("checks"), checks),
    ])
}

/// A single-entry table reporting the loaded recipe count.
pub fn count_value(cx: &mut Cx, count: usize) -> Result<Value> {
    let recipes = string(cx, &count.to_string())?;
    cx.factory().table(vec![(key("recipes"), recipes)])
}
