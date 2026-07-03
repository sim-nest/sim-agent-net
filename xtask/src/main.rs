#![forbid(unsafe_code)]

mod recipe_assertions;
#[cfg(test)]
mod recipe_assertions_tests;
mod simdoc;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

// Run the agent-net recipe-corpus assertions, then the shared simdoc task. The
// assertions are agent-net specific, so they live here rather than in the
// `simdoc` module, which is kept byte-identical to the canonical sim-tooling copy
// (see `simctl sync-xtask`).
fn run() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|err| format!("current dir: {err}"))?;
    let summary = recipe_assertions::check_repo(&root)?;
    println!(
        "simdoc: recipe assertions checked {} recipe(s), including {} 30-agents recipe(s)",
        summary.checked_recipes, summary.agent30_recipes
    );
    simdoc::run(std::env::args().collect())
}
