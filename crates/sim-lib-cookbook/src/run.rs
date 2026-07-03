//! The `cookbook:run` algorithm: decode a recipe's setup through its declared
//! codec, evaluate it, encode the result, and check declared expectations.
//!
//! Per the design (Section 7) this is capability-gated: running a recipe is
//! read-eval, so the runtime must hold the read-eval capability. Listing and
//! showing recipes is not gated.
//!
//! First cut: the setup decodes to a single top-level form (form 0). The data
//! model already carries a `Vec` of results and expectations, so multi-form
//! setups are a forward-compatible extension.

use sim_codec::{Input, decode_with_codec, encode_value_with_codec};
use sim_cookbook::{CheckResult, RecipeCard, RecipeRun};
use sim_kernel::{Cx, EncodeOptions, Error, ReadPolicy, Result, Symbol, read_eval_capability};

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

/// Run a recipe end to end. Hard errors (missing requires, unknown codec,
/// undecodable setup) return `Err`; an evaluation error is captured as
/// `ok == false` with empty results so the caller still sees a `RecipeRun`.
pub fn run_recipe(cx: &mut Cx, card: &RecipeCard) -> Result<RecipeRun> {
    let missing = missing_requires(cx, card);
    if !missing.is_empty() {
        return Err(Error::Eval(format!(
            "recipe {} requires libs not loaded: {}",
            card.id,
            missing.join(", ")
        )));
    }

    let codec = Symbol::qualified("codec", card.codec.as_str());
    let source = String::from_utf8(card.setup.clone())
        .map_err(|e| Error::Eval(format!("recipe {} setup is not UTF-8: {e}", card.id)))?;
    let expr = decode_with_codec(cx, &codec, Input::Text(source), ReadPolicy::default())?;

    let (results, eval_ok) = match cx.eval_expr(expr) {
        Ok(value) => {
            let text = encode_value_with_codec(cx, &codec, &value, EncodeOptions::default())?
                .into_text()?;
            (vec![text], true)
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

/// Decode a recipe's setup to an `Expr` without evaluating it (`cookbook:setup`).
pub fn decode_setup(cx: &mut Cx, card: &RecipeCard) -> Result<sim_kernel::Expr> {
    let codec = Symbol::qualified("codec", card.codec.as_str());
    let source = String::from_utf8(card.setup.clone())
        .map_err(|e| Error::Eval(format!("recipe {} setup is not UTF-8: {e}", card.id)))?;
    decode_with_codec(cx, &codec, Input::Text(source), ReadPolicy::default())
}
