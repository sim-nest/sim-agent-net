//! The `cookbook:run` algorithm: decode a recipe's setup through its declared
//! codec, evaluate it, encode the result, and check declared expectations.
//!
//! Per the design (Section 7) this is capability-gated: running a recipe is
//! read-eval, so the runtime must hold the read-eval capability. Listing and
//! showing recipes is not gated.
//!
//! Each setup decodes to a single top-level form. The data model carries a
//! `Vec` of results and expectations, and this runner fills it from that
//! decoded form without changing the stored recipe shape.

use sim_codec::{
    Input, decode_eval_expr_with_codec, decode_with_codec, encode_value_with_codec,
    lower_operator_nodes,
};
use sim_cookbook::{CheckResult, RecipeCard, RecipeRun};
use sim_kernel::{
    CapabilitySet, Cx, EncodeOptions, Error, ReadPolicy, Result, Symbol, TrustLevel,
    read_construct_capability, read_eval_capability,
};

use crate::catalog::{EmptyCatalog, LibCatalog, load_requires};

/// Error unless the runtime holds the read-eval capability.
pub fn require_eval_capability(cx: &Cx) -> Result<()> {
    if cx.capabilities().contains(&read_eval_capability()) {
        Ok(())
    } else {
        Err(Error::CapabilityDenied {
            capability: read_eval_capability(),
        })
    }
}

/// Lib ids in `card.requires` that are not currently loaded.
///
/// A requirement matches a loaded lib by its fully qualified id
/// (`namespace/name`) or by its unqualified `name` alone, so `numbers-f64`
/// matches a lib whose id is `lisp/numbers-f64`.
pub fn missing_requires(cx: &Cx, card: &RecipeCard) -> Vec<String> {
    let loaded: Vec<(String, String)> = cx
        .registry()
        .libs()
        .iter()
        .map(|lib| {
            (
                lib.manifest.id.as_qualified_str(),
                lib.manifest.id.name.to_string(),
            )
        })
        .collect();
    card.requires
        .iter()
        .filter(|req| {
            !loaded
                .iter()
                .any(|(qualified, name)| qualified == *req || name == *req)
        })
        .cloned()
        .collect()
}

/// Run a recipe end to end against an [`EmptyCatalog`] (the direct path): every
/// required lib must already be loaded into `cx`, or the run errors.
///
/// Hard errors (missing requires, unknown codec, undecodable setup) return
/// `Err`; an evaluation error is captured as `ok == false` with empty results so
/// the caller still sees a `RecipeRun`.
pub fn run_recipe(cx: &mut Cx, card: &RecipeCard) -> Result<RecipeRun> {
    run_recipe_with_catalog(cx, &EmptyCatalog, card)
}

/// Run a recipe end to end, loading its `requires` from `catalog` first.
///
/// Before decode+eval the runner asks `catalog` to resolve each `requires` entry
/// and loads the returned lib into the eval `Cx`, idempotently. A require the
/// catalog does not carry (and that is not already loaded) makes the recipe a
/// descriptor: the run returns `Err(Error::Eval("descriptor: requires <x> not in
/// catalog"))`. This is what turns runnability into a structural property of
/// (catalog + capability profile) rather than a hand-applied label.
pub fn run_recipe_with_catalog(
    cx: &mut Cx,
    catalog: &dyn LibCatalog,
    card: &RecipeCard,
) -> Result<RecipeRun> {
    require_eval_capability(cx)?;

    let unresolved = load_requires(cx, catalog, card);
    if !unresolved.is_empty() {
        return Err(Error::Eval(format!(
            "recipe {} descriptor: requires not in catalog: {}",
            card.id,
            unresolved.join(", ")
        )));
    }

    let codec = Symbol::qualified("codec", card.codec.as_str());
    let source = String::from_utf8(card.setup.clone())
        .map_err(|e| Error::Eval(format!("recipe {} setup is not UTF-8: {e}", card.id)))?;
    // Recipes are trusted embedded content; grant the read the capabilities their setup
    // needs -- read-construct so `#(Class ...)` builds a real domain value (complex,
    // tensor, rational, CAS), and read-eval for eval-position reader forms.
    // `ReadPolicy::default()` grants neither, which is why domain recipes could only quote.
    let read_policy = ReadPolicy {
        trust: TrustLevel::TrustedSource,
        capabilities: CapabilitySet::new()
            .grant(read_construct_capability())
            .grant(read_eval_capability()),
    };
    // Decode to an evaluable expression without a Term/Datum round-trip. The
    // shared lowerer turns Algol-style operator nodes into calls while keeping
    // Lisp special-form list containers structurally intact.
    let expr = lower_operator_nodes(decode_eval_expr_with_codec(
        cx,
        &codec,
        Input::Text(source),
        read_policy,
    )?);

    let (results, eval_ok) = match cx.eval_expr(expr) {
        Ok(value) => {
            // Encode the computed value back with the recipe's own codec so a
            // round-tripping surface (lisp, json, algol) reports on its own
            // surface. A decode-only language codec (e.g. scheme-r7rs-small, which
            // parses its surface but has no encoder) cannot render the result, so
            // fall back to the canonical `codec/lisp` display -- the setup still
            // parsed and evaluated on its own surface.
            let encoded = encode_value_with_codec(cx, &codec, &value, EncodeOptions::default())
                .or_else(|_| {
                    let lisp = Symbol::qualified("codec", "lisp");
                    encode_value_with_codec(cx, &lisp, &value, EncodeOptions::default())
                })?;
            (vec![encoded.into_text()?], true)
        }
        Err(_) => (Vec::new(), false),
    };

    let mut checks = Vec::new();
    let mut all_pass = true;
    for expectation in &card.expect {
        let actual = results.get(expectation.form).cloned();
        let pass = actual.as_deref() == Some(expectation.result.as_str());
        if !pass {
            all_pass = false;
        }
        checks.push(CheckResult {
            form: expectation.form,
            expected: expectation.result.clone(),
            actual: actual.unwrap_or_else(|| "<no such form>".to_string()),
            pass,
        });
    }

    Ok(RecipeRun {
        recipe: card.id.clone(),
        forms: results.len(),
        results,
        ok: eval_ok && all_pass,
        checks,
    })
}

/// Run a Category C recipe twice under the same (catalog + Cx) and confirm the
/// two runs produce identical results.
///
/// Floating-point audio/FEM results drift across platforms, so Category C
/// results are encoded as deterministic artifacts (digests, frames). Running the
/// recipe twice and asserting the results match is a cheap, strong catch for an
/// entropy or wall-clock leak: a non-deterministic recipe returns
/// `Err(Error::Eval("... not deterministic ..."))`. The first run's `RecipeRun`
/// is returned on success.
pub fn run_recipe_twice(
    cx: &mut Cx,
    catalog: &dyn LibCatalog,
    card: &RecipeCard,
) -> Result<RecipeRun> {
    let first = run_recipe_with_catalog(cx, catalog, card)?;
    let second = run_recipe_with_catalog(cx, catalog, card)?;
    if first.results != second.results {
        return Err(Error::Eval(format!(
            "recipe {} is not deterministic: {:?} != {:?}",
            card.id, first.results, second.results
        )));
    }
    Ok(first)
}

/// Decode a recipe's setup to an `Expr` without evaluating it (`cookbook:setup`).
pub fn decode_setup(cx: &mut Cx, card: &RecipeCard) -> Result<sim_kernel::Expr> {
    let codec = Symbol::qualified("codec", card.codec.as_str());
    let source = String::from_utf8(card.setup.clone())
        .map_err(|e| Error::Eval(format!("recipe {} setup is not UTF-8: {e}", card.id)))?;
    decode_with_codec(cx, &codec, Input::Text(source), ReadPolicy::default())
}
