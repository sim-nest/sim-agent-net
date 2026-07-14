use crate::{stringish_from_value, value_from_expr};
use sim_codec::{Input, decode_with_codec};
use sim_cookbook::{Expectation, parse_recipe};
use sim_kernel::{Args, Cx, Error, ReadPolicy, Result, Symbol, Value};
use std::str;

pub(crate) fn agent_recipe_result_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let recipe_id = match args.into_vec().as_slice() {
        [recipe_id] => stringish_from_value(
            cx,
            recipe_id.clone(),
            "agent/recipe-result expects a recipe id",
        )?,
        _ => {
            return Err(Error::Eval(
                "agent/recipe-result expects one recipe id".to_owned(),
            ));
        }
    };

    let mut matching_recipes = Vec::new();
    for (path, bytes) in crate::RECIPES
        .iter()
        .filter(|(path, _)| path.ends_with("/recipe.toml"))
    {
        let recipe = recipe_from_embedded(path, bytes).map_err(Error::Eval)?;
        if recipe_id_matches(&recipe.full_id, &recipe.manifest.id, &recipe_id) {
            matching_recipes.push(recipe);
        }
    }
    let recipe = match matching_recipes.as_slice() {
        [recipe] => recipe,
        [] => {
            return Err(Error::Eval(format!(
                "agent/recipe-result could not find recipe {recipe_id}"
            )));
        }
        _ => {
            return Err(Error::Eval(format!(
                "agent/recipe-result matched multiple recipes for {recipe_id}"
            )));
        }
    };

    let expectation = match recipe.manifest.expect.as_slice() {
        [expectation] if expectation.form == 0 => expectation,
        [] => {
            return Err(Error::Eval(format!(
                "agent/recipe-result recipe {} has no expected result",
                recipe.full_id
            )));
        }
        [expectation] => {
            return Err(Error::Eval(format!(
                "agent/recipe-result recipe {} expects form {}, not form 0",
                recipe.full_id, expectation.form
            )));
        }
        _ => {
            return Err(Error::Eval(format!(
                "agent/recipe-result recipe {} has multiple expected results",
                recipe.full_id
            )));
        }
    };

    let expr = decode_expectation(cx, &recipe.manifest.codec, expectation)?;
    value_from_expr(cx, &expr)
}

struct EmbeddedRecipe {
    full_id: String,
    manifest: sim_cookbook::RecipeManifest,
}

fn recipe_from_embedded(path: &str, bytes: &[u8]) -> std::result::Result<EmbeddedRecipe, String> {
    let text = str::from_utf8(bytes).map_err(|err| format!("{path}: {err}"))?;
    let manifest = parse_recipe(text).map_err(|err| format!("{path}: {err}"))?;
    let chapter = path
        .split('/')
        .next()
        .ok_or_else(|| format!("{path}: missing chapter"))?;
    Ok(EmbeddedRecipe {
        full_id: format!("agent/{chapter}/{}", manifest.id),
        manifest,
    })
}

fn recipe_id_matches(full_id: &str, local_id: &str, requested: &str) -> bool {
    full_id == requested || local_id == requested
}

fn decode_expectation(
    cx: &mut Cx,
    codec: &str,
    expectation: &Expectation,
) -> Result<sim_kernel::Expr> {
    let codec = Symbol::qualified("codec", codec);
    decode_with_codec(
        cx,
        &codec,
        Input::Text(expectation.result.clone()),
        ReadPolicy::default(),
    )
}
